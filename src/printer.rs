pub mod usb;
// NIIMBOT N1 native USB serial and BLE transports and single-page print task.
// Wire behavior adapted from iscarelli/niimbot-web-bluetooth 2.6.0 (MIT);
// see THIRD-PARTY-NIIMBOT.txt and https://www.npmjs.com/package/niimbot-web-bluetooth/v/2.6.0.
use btleplug::{
    api::{
        Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
        ValueNotification, WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use futures::{FutureExt, Stream, StreamExt};
use std::{collections::VecDeque, pin::Pin, time::Duration};
use tokio::time::{sleep, timeout, timeout_at, Instant};
use uuid::Uuid;

const SERVICE: Uuid = Uuid::from_u128(0xe781_0a71_73ae_499d_8c15_faa9_aef0_c3f2);
const CHARACTERISTIC: Uuid = Uuid::from_u128(0xbef8_d6c9_9c21_4c9e_b632_bd58_c100_9f9f);

const MODEL_ID: u16 = 3586;

const STRIDE: usize = 12;

struct Ble {
    disconnect_monitor: Option<crate::ble_connection::DisconnectMonitor>,
    peripheral: Peripheral,
    characteristic: Characteristic,
    notify: Pin<Box<dyn Stream<Item = ValueNotification> + Send>>,
    write_type: WriteType,
}

enum Link {
    Ble(Ble),
    Usb(usb::Usb),
    Closed,
}

pub struct Printer {
    link: Link,
    name: String,
    ready: bool,
    decoder: Decoder,
    replies: VecDeque<Reply>,
}

#[derive(Debug, PartialEq, Eq)]
struct Reply {
    command: u8,
    body: Vec<u8>,
}

fn frame(cmd: u8, body: &[u8]) -> Vec<u8> {
    assert!(body.len() <= u8::MAX as usize);
    let len = body.len() as u8;
    let check = body.iter().fold(cmd ^ len, |a, b| a ^ b);
    let mut out = vec![0x55, 0x55, cmd, len];
    out.extend_from_slice(body);
    out.extend_from_slice(&[check, 0xaa, 0xaa]);
    out
}

#[derive(Default)]
struct Decoder {
    bytes: Vec<u8>,
}

impl Decoder {
    fn push(&mut self, bytes: &[u8], replies: &mut VecDeque<Reply>) {
        self.bytes.extend_from_slice(bytes);
        loop {
            let Some(head) = self.bytes.windows(2).position(|w| w == [0x55, 0x55]) else {
                let trailing_start = self.bytes.last() == Some(&0x55);
                self.bytes.clear();
                if trailing_start {
                    self.bytes.push(0x55);
                }
                break;
            };
            self.bytes.drain(..head);
            if self.bytes.len() < 4 {
                break;
            }
            let end = self.bytes[3] as usize + 7;
            if self.bytes.len() < end {
                break;
            }
            let check = self.bytes[2..end - 3].iter().fold(0, |a, b| a ^ b);
            if self.bytes[end - 3..end] == [check, 0xaa, 0xaa] {
                replies.push_back(Reply {
                    command: self.bytes[2],
                    body: self.bytes[4..end - 3].to_vec(),
                });
                self.bytes.drain(..end);
            } else {
                self.bytes.drain(..1);
            }
        }
    }
}

fn model_id(reply: &Reply) -> Option<u16> {
    if reply.command != 0x48 {
        return None;
    }
    Some(u16::from_be_bytes([
        *reply.body.first()?,
        *reply.body.get(1)?,
    ]))
}

// Unbundled run-length packets, 200 runs max, each row contains 96 MSB-first dots.
fn encode_rows(buf: &[u8]) -> Vec<Vec<u8>> {
    let height = buf.len() / STRIDE;
    let mut out = Vec::new();
    let mut row = 0;
    while row < height {
        let current = &buf[row * STRIDE..(row + 1) * STRIDE];
        let run = (1..200.min(height - row))
            .take_while(|next| current == &buf[(row + next) * STRIDE..(row + next + 1) * STRIDE])
            .count()
            + 1;
        let [hi, lo] = (row as u16).to_be_bytes();
        if current.iter().all(|&v| v == 0) {
            out.push(frame(0x84, &[hi, lo, run as u8]));
        } else {
            let count = current.iter().map(|v| v.count_ones() as u8).sum();
            let mut body = vec![hi, lo, 0, count, 0, run as u8];
            body.extend_from_slice(current);
            out.push(frame(0x85, &body));
        }
        row += run;
    }
    out
}

fn tag_barcode(body: &[u8]) -> Option<&str> {
    let (len, fields) = body.get(8..)?.split_first()?;
    let (barcode, tail) = fields.split_at_checked(usize::from(*len))?;
    let (serial_len, tail) = tail.split_first()?;
    tail.get(usize::from(*serial_len)..)?.get(..5)?;
    let barcode = std::str::from_utf8(barcode).ok()?;
    (!barcode.is_empty() && !barcode.chars().any(char::is_control)).then_some(barcode)
}

impl Ble {
    async fn write(&self, buf: &[u8]) -> Result<(), String> {
        timeout(
            Duration::from_secs(2),
            self.peripheral
                .write(&self.characteristic, buf, self.write_type),
        )
        .await
        .map_err(|_| "写入打印机超时。".to_string())?
        .map_err(|_| "蓝牙写入失败，请检查打印机连接。".to_string())?;
        if self.write_type == WriteType::WithoutResponse {
            sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }
}

impl Printer {
    pub async fn connect() -> Result<Self, String> {
        let manager = Manager::new()
            .await
            .map_err(|_| "无法启用系统蓝牙，请检查权限和蓝牙服务。".to_string())?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|_| "无法读取蓝牙适配器。".to_string())?;
        let mut candidates = Vec::new();
        let mut scanned = false;
        for adapter in adapters {
            if timeout(
                Duration::from_secs(4),
                adapter.start_scan(ScanFilter::default()),
            )
            .await
            .is_ok_and(|r| r.is_ok())
            {
                scanned = true;
                // N1 can omit the service UUID from advertisements; filter by name.
                sleep(Duration::from_secs(4)).await;
                let devices = adapter.peripherals().await.unwrap_or_default();
                let _ = timeout(Duration::from_secs(2), adapter.stop_scan()).await;
                for peripheral in devices {
                    let Ok(Some(properties)) = peripheral.properties().await else {
                        continue;
                    };
                    let Some(name) = properties.local_name else {
                        continue;
                    };
                    if name == "N1" || name.starts_with("N1-") || name.starts_with("N1_") {
                        candidates.push((adapter.clone(), peripheral, name));
                    }
                }
            }
        }
        let (adapter, peripheral, name) = match candidates.len() {
            0 if !scanned => return Err("蓝牙扫描不可用，请确认系统蓝牙已开启。".into()),
            0 => return Err("未找到 N1，请打开打印机并让手机 App 断开蓝牙。".into()),
            1 => candidates.pop().unwrap(),
            _ => return Err("发现多台 N1，请只开启要连接的一台后重试。".into()),
        };
        let result = timeout(
            Duration::from_secs(16),
            Self::open(adapter, peripheral.clone(), name),
        )
        .await;
        match result {
            Ok(Ok(printer)) => Ok(printer),
            Ok(Err(reason)) => {
                let _ = timeout(Duration::from_secs(3), peripheral.disconnect()).await;
                Err(reason)
            }
            Err(_) => {
                let _ = timeout(Duration::from_secs(3), peripheral.disconnect()).await;
                Err("连接 N1 超时，请靠近打印机并重新连接。".into())
            }
        }
    }

    async fn open(adapter: Adapter, peripheral: Peripheral, name: String) -> Result<Self, String> {
        peripheral
            .connect()
            .await
            .map_err(|_| "无法建立蓝牙连接。")?;
        let monitor = crate::ble_connection::disconnect_monitor(adapter, peripheral.clone()).await;
        peripheral
            .discover_services()
            .await
            .map_err(|_| "无法读取打印机蓝牙服务。")?;
        let characteristic = peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == CHARACTERISTIC && c.service_uuid == SERVICE)
            .ok_or("设备未提供 N1 打印服务。")?;
        let props = characteristic.properties;
        let write_type = if props.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
            WriteType::WithoutResponse
        } else if props.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            return Err("N1 打印服务不可写入。".into());
        };
        // Create the notification stream first so an early response cannot be lost.
        let notify = peripheral
            .notifications()
            .await
            .map_err(|_| "无法建立打印机通知通道。")?;
        peripheral
            .subscribe(&characteristic)
            .await
            .map_err(|_| "无法接收打印机通知。")?;
        Self::identify(
            Link::Ble(Ble {
                disconnect_monitor: Some(monitor),
                peripheral,
                characteristic,
                notify,
                write_type,
            }),
            name,
        )
        .await
    }

    pub async fn connect_usb(path: String) -> Result<Self, String> {
        let name = format!("N1 USB ({path})");
        Self::identify(Link::Usb(usb::Usb::open(path).await?), name).await
    }

    async fn identify(link: Link, name: String) -> Result<Self, String> {
        let mut this = Self {
            link,
            name,
            ready: false,
            decoder: Decoder::default(),
            replies: VecDeque::new(),
        };
        let mut init = vec![0x03];
        init.extend(frame(0xc1, &[0x01]));
        this.write(&init).await?;
        sleep(Duration::from_millis(200)).await;
        // N1 may answer StatusData with B4 rather than B5; model ID is authoritative.
        let _ = this
            .exchange(0xa5, &[0x01], &[0xb4, 0xb5], Duration::from_secs(1))
            .await;
        let model = this
            .exchange(0x40, &[0x08], &[0x48], Duration::from_secs(1))
            .await
            .ok()
            .and_then(|r| model_id(&r));
        if model != Some(MODEL_ID) {
            return Err("未确认设备型号为精臣 N1，未开启打印。".into());
        }
        // B1-task handshake, using read/keepalive operations only.
        let _ = this
            .exchange(0xa5, &[0x01], &[0xb4, 0xb5], Duration::from_secs(1))
            .await;
        for sub in [0x08, 0x0b, 0x0d, 0x0a, 0x07, 0x03, 0x0c, 0x09] {
            let _ = this
                .exchange(
                    0x40,
                    &[sub],
                    &[
                        0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c,
                        0x4d, 0x4e, 0x4f,
                    ],
                    Duration::from_millis(600),
                )
                .await;
        }
        let _ = this
            .exchange(0xdc, &[0x04], &[0xd9], Duration::from_secs(1))
            .await;
        if !this.link_connected().await {
            return Err("连接中断，请重新连接 N1。".into());
        }
        this.ready = true;
        Ok(this)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn read_stock_barcode(&mut self) -> Option<String> {
        let reply = self
            .exchange(0x1a, &[1], &[0x1b], Duration::from_secs(1))
            .await
            .ok()?;
        tag_barcode(&reply.body).map(str::to_owned)
    }

    pub fn take_disconnect_monitor(&mut self) -> Option<crate::ble_connection::DisconnectMonitor> {
        match &mut self.link {
            Link::Ble(ble) => ble.disconnect_monitor.take(),
            _ => None,
        }
    }

    pub async fn is_connected(&self) -> bool {
        self.ready && self.link_connected().await
    }

    async fn link_connected(&self) -> bool {
        match &self.link {
            Link::Usb(port) => port.connected(),
            Link::Ble(ble) => {
                crate::ble_connection::connection_state(&ble.peripheral).await == Some(true)
            }
            Link::Closed => false,
        }
    }

    pub async fn disconnect(&mut self) -> Result<(), String> {
        let Link::Ble(ble) = std::mem::replace(&mut self.link, Link::Closed) else {
            return Ok(());
        };
        timeout(Duration::from_secs(4), ble.peripheral.disconnect())
            .await
            .map_err(|_| "断开蓝牙连接超时。".to_string())?
            .map_err(|_| "无法断开蓝牙连接。".to_string())
    }

    async fn write(&self, buf: &[u8]) -> Result<(), String> {
        let ble = match &self.link {
            Link::Usb(port) => return port.send(buf.to_vec(), None).await.map(|_| ()),
            Link::Ble(ble) => ble,
            Link::Closed => return Err("打印机已断开连接。".into()),
        };
        ble.write(buf).await
    }

    async fn exchange(
        &mut self,
        cmd: u8,
        body: &[u8],
        expected: &[u8],
        limit: Duration,
    ) -> Result<Reply, String> {
        let ble = match &mut self.link {
            Link::Usb(port) => {
                return port
                    .send(frame(cmd, body), Some((expected.to_vec(), limit)))
                    .await?
                    .ok_or_else(|| "缺少 USB 回复。".into())
            }
            Link::Ble(ble) => ble,
            Link::Closed => return Err("打印机已断开连接。".into()),
        };
        // Drop already queued notifications so a previous job's status cannot count.
        self.replies.clear();
        self.decoder.bytes.clear();
        for _ in 0..128 {
            if ble.notify.next().now_or_never().flatten().is_none() {
                break;
            }
        }
        ble.write(&frame(cmd, body)).await?;
        let deadline = Instant::now() + limit;
        loop {
            while let Some(reply) = self.replies.pop_front() {
                if expected.contains(&reply.command) {
                    return Ok(reply);
                }
            }
            let notice = timeout_at(deadline, ble.notify.next())
                .await
                .map_err(|_| "打印机未在预期时间内响应。".to_string())?
                .ok_or_else(|| "打印机已断开连接。".to_string())?;
            if notice.uuid == CHARACTERISTIC {
                self.decoder.push(&notice.value, &mut self.replies);
            }
        }
    }

    pub async fn print_page(&mut self, buf: &[u8]) -> Result<(), String> {
        if buf.len() != 200 * STRIDE {
            return Err("N1 打印点阵必须为 96 × 200。".into());
        }
        if !self.is_connected().await {
            return Err("请重新连接已识别的 N1。".into());
        }

        // No retry for physical operations; timeout may mean response loss, not
        // command loss. In that case require explicit reconnect before another job.
        if self
            .exchange(0x21, &[3], &[0x31], Duration::from_secs(1))
            .await
            .is_err()
            || self
                .exchange(0x23, &[1], &[0x33], Duration::from_secs(1))
                .await
                .is_err()
        {
            self.ready = false;
            let _ = self.disconnect().await;
            return Err("打印机未确认耗材设置，请检查设备并重连。".into());
        }

        let page_result = self.send_page(buf).await;
        // Once PrintStart is attempted, best effort PrintEnd even after timeout, but
        // do not claim physical completion unless all required responses arrived.
        let ended = self
            .exchange(0xf3, &[1], &[0xf4], Duration::from_millis(2500))
            .await;
        match (page_result, ended) {
            (Ok(()), Ok(_)) => Ok(()),
            (page, end) => {
                self.ready = false;
                let _ = self.disconnect().await;
                let stage = if page.is_err() {
                    "未确认当前标签完成"
                } else if end.is_err() {
                    "未收到任务结束确认"
                } else {
                    "打印未确认"
                };
                Err(format!(
                    "{stage}；可能已出纸，请核对后再连接，避免重复打印。"
                ))
            }
        }
    }

    async fn send_page(&mut self, buf: &[u8]) -> Result<(), String> {
        self.exchange(
            0x01,
            &[0, 1, 0, 0, 0, 0, 0],
            &[0x02],
            Duration::from_secs(2),
        )
        .await?;
        self.exchange(0x03, &[1], &[0x04], Duration::from_secs(1))
            .await?;
        let [hi, lo] = ((buf.len() / STRIDE) as u16).to_be_bytes();
        self.exchange(
            0x13,
            &[hi, lo, 0, 0x60, 0, 1],
            &[0x14],
            Duration::from_secs(2),
        )
        .await?;
        for row in encode_rows(buf) {
            self.write(&row).await?;
        }
        self.exchange(0xe3, &[1], &[0xe4], Duration::from_secs(10))
            .await?;
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            let status = self
                .exchange(0xa3, &[1], &[0xb3], Duration::from_millis(900))
                .await;
            if status.is_ok_and(|s| s.body.len() >= 4 && s.body[..2] == [0, 1]) {
                return Ok(());
            }
            sleep(Duration::from_millis(150)).await;
        }
        Err("打印机未确认该张标签已打印。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_matches_connect_and_mixed_stream_decodes() {
        assert_eq!(
            frame(0xc1, &[1]),
            [0x55, 0x55, 0xc1, 1, 1, 0xc1, 0xaa, 0xaa]
        );
        let model = frame(0x48, &[0x0e, 0x02]);
        let mut broken = frame(0x48, &[0x01, 0x02]);
        broken[6] ^= 0xff;
        let mut parser = Decoder::default();
        let mut output = VecDeque::new();
        parser.push(&[0x03, 0x55], &mut output);
        parser.push(&broken[1..], &mut output);
        parser.push(&model[..4], &mut output);
        let chunk = [model[4..].to_vec(), frame(0xe4, &[1])].concat();
        parser.push(&chunk, &mut output);
        assert_eq!(output.len(), 2);
        assert_eq!(model_id(&output.pop_front().unwrap()), Some(MODEL_ID));
        assert_eq!(output.pop_front().unwrap().command, 0xe4);
    }

    #[test]
    fn white_runs_and_mixed_rows_encode_absolute_row_and_popcount() {
        let mut image = [0; 400 * STRIDE];
        assert_eq!(
            encode_rows(&image),
            [frame(0x84, &[0, 0, 200]), frame(0x84, &[0, 200, 200])]
        );
        image[STRIDE * 257] = 0b1110_0000;
        image[STRIDE * 258] = 0b1110_0000;
        let output = encode_rows(&image);
        assert_eq!(output.len(), 4);
        assert_eq!(
            output[2],
            frame(
                0x85,
                &[1, 1, 0, 3, 0, 2, 0xe0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
            )
        );
        assert_eq!(output[3], frame(0x84, &[1, 3, 141]));
    }
}

#[cfg(test)]
#[test]
fn decodes_stock_id_only_from_complete_tag() {
    let body = b"12345678\x03012\x02SN\0\x64\0\x01\x01";
    assert_eq!(tag_barcode(body), Some("012"));
    for n in 0..body.len() {
        assert_eq!(tag_barcode(&body[..n]), None);
    }
    assert_eq!(tag_barcode(&[0]), None);
}

#[test]
#[cfg(test)]
fn silver_roll_uses_200_feed_rows() {
    use crate::{
        core::{builtin_media, render_svg, Row, Template},
        stock::Model,
    };
    let (_, stock, _) = builtin_media()
        .iter()
        .find(|(m, _, codes)| *m == Model::N1 && codes.contains(&"05262201"))
        .unwrap();
    assert_eq!(stock.canvas_size(), (200, 96));
    let page = render_svg(&Template::n1_example(stock.clone()), &Row::new(), "0001").unwrap();
    let packed = crate::raster::packed_n1(&page, stock).unwrap();
    assert_eq!(packed.len(), 2400);
    assert_eq!(encode_rows(&vec![0; 2400]), vec![frame(0x84, &[0, 0, 200])]);
}
