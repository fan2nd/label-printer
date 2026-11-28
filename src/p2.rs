//! DeTong P2 BLE printing. The wire format is described in IvanLi-CN/detonger
//! (docs/ble.md and docs/protocol.md). A successful write is a submission only.
mod ble_page;
pub mod usb;
use btleplug::{
    api::{
        Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
        WriteType,
    },
    platform::{Manager, Peripheral},
};
use std::time::Duration;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

const SERVICE: Uuid = Uuid::from_u128(0x49535343_fe7d_4ae5_8fa9_9fafd205e455);
const WRITE: Uuid = Uuid::from_u128(0x49535343_8841_43f4_a8d4_ecbe34729bb3);
// The second endpoint is advertised by a P2 measured on macOS.
const WRITE_ENDPOINTS: [(Uuid, Uuid); 2] = [
    (SERVICE, WRITE),
    (
        uuid::uuid!("0000ff00-0000-1000-8000-00805f9b34fb"),
        uuid::uuid!("0000ff02-0000-1000-8000-00805f9b34fb"),
    ),
];
const STRIDE: usize = 48;
const MAX_ROWS: usize = 1600;

// Setup commands for one die-cut label.
pub const DEFAULT_DENSITY: u8 = 1;
const SETUP: [&[u8]; 4] = [
    &[0x1f, 0x20, 2, 0, 1, 0x88], // start page 1
    &[0x1f, 0x27, 1, STRIDE as u8, 0x88],
    &[0x1f, 0x42, 1, 2, 0x88],          // gap media
    &[0x1f, 0x45, 2, 0xff, 0xff, 0x88], // automatic gap length
];

fn density_command(level: u8) -> Result<[u8; 5], String> {
    if !(1..=15).contains(&level) {
        return Err("P2 打印浓度须为 1～15。".into());
    }
    Ok([0x1f, 0x43, 1, level - 1, 0x88])
}

fn validate_bitmap(pixels: &[u8]) -> Result<(), String> {
    if pixels.is_empty() || pixels.len() % STRIDE != 0 || pixels.len() > STRIDE * MAX_ROWS {
        return Err("P2 图像尺寸无效，请检查标签规格。".into());
    }
    Ok(())
}

// ESC J advances blank raster lines;
// 1F 2B takes a leading byte offset and pads the remainder of the row white.
fn bitmap_messages(pixels: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    validate_bitmap(pixels)?;
    let mut rows = pixels.chunks_exact(STRIDE).peekable();
    let mut messages = Vec::new();
    while let Some(row) = rows.next() {
        if let Some(message) = ink_command(row) {
            messages.push(message);
        } else {
            let mut blank = 1u8;
            while blank < u8::MAX && rows.next_if(|r| r.iter().all(|&byte| byte == 0)).is_some() {
                blank += 1;
            }
            messages.push(vec![0x1b, 0x4a, blank]);
        }
    }
    messages.last_mut().unwrap().push(0x0c);
    Ok(messages)
}

fn ink_command(row: &[u8]) -> Option<Vec<u8>> {
    let first = row.iter().position(|&b| b != 0)?;
    let last = row.iter().rposition(|&b| b != 0).unwrap();
    let ink = &row[first..=last];
    let mut cmd = vec![0x1f, 0x2b, first as u8, ink.len() as u8];
    cmd.extend_from_slice(ink);
    Some(cmd)
}

fn is_p2(name: &str) -> bool {
    let matches_model = |n: &str| matches!(n.trim().split(['-', '_']).next(), Some("P2"));
    matches_model(name)
        || name
            .rsplit_once(" [")
            .and_then(|(_, ad)| ad.strip_suffix(']'))
            .is_some_and(matches_model)
}

pub struct Printer {
    disconnect_monitor: Option<crate::ble_connection::DisconnectMonitor>,
    peripheral: Peripheral,
    characteristic: Characteristic,
    mode: WriteType,
    name: String,
    usable: bool,
}

impl Printer {
    pub async fn connect() -> Result<Self, String> {
        let manager = timeout(Duration::from_secs(5), Manager::new())
            .await
            .map_err(|_| "启用蓝牙超时。")?
            .map_err(|_| "无法启用蓝牙，请检查系统权限。")?;
        let adapters = timeout(Duration::from_secs(5), manager.adapters())
            .await
            .map_err(|_| "读取蓝牙适配器超时。")?
            .map_err(|_| "无法读取蓝牙适配器。")?;
        let mut found = Vec::new();
        let mut scanned = false;
        for adapter in adapters {
            if !timeout(
                Duration::from_secs(5),
                adapter.start_scan(ScanFilter::default()),
            )
            .await
            .is_ok_and(|r| r.is_ok())
            {
                continue;
            }
            scanned = true;
            sleep(Duration::from_secs(7)).await;
            let devices = timeout(Duration::from_secs(3), adapter.peripherals()).await;
            let _ = timeout(Duration::from_secs(2), adapter.stop_scan()).await;
            let Ok(Ok(devices)) = devices else { continue };
            for peripheral in devices {
                if let Ok(Ok(Some(props))) =
                    timeout(Duration::from_secs(2), peripheral.properties()).await
                {
                    if let Some(name) = props.local_name.filter(|name| is_p2(name)) {
                        found.push((adapter.clone(), peripheral, name));
                    }
                }
            }
        }
        if !scanned {
            return Err("无法扫描 P2，请开启系统蓝牙并检查权限。".into());
        }
        if found.len() > 1 {
            return Err("发现多台 P2，请只开启目标打印机后重新连接。".into());
        }
        let (adapter, peripheral, name) = found
            .pop()
            .ok_or("未找到 P2，请开启打印机并断开其他应用的连接。")?;
        let setup = async {
            peripheral.connect().await.map_err(|_| "无法连接 P2。")?;
            let monitor =
                crate::ble_connection::disconnect_monitor(adapter, peripheral.clone()).await;
            peripheral
                .discover_services()
                .await
                .map_err(|_| "无法读取 P2 蓝牙服务。")?;
            let characteristics = peripheral.characteristics();
            for (service, write) in WRITE_ENDPOINTS {
                if let Some(c) = characteristics
                    .iter()
                    .find(|c| c.service_uuid == service && c.uuid == write)
                {
                    let flags = c.properties;
                    if service != SERVICE && flags.contains(CharPropFlags::WRITE) {
                        return Ok((c.clone(), WriteType::WithResponse, monitor));
                    }
                    if flags.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
                        return Ok((c.clone(), WriteType::WithoutResponse, monitor));
                    }
                    if flags.contains(CharPropFlags::WRITE) {
                        return Ok((c.clone(), WriteType::WithResponse, monitor));
                    }
                }
            }
            Err("P2 已连接，但未发现当前驱动支持的可写打印特征。")
        };
        let ready = timeout(Duration::from_secs(16), setup)
            .await
            .unwrap_or(Err("连接 P2 超时，请重新连接。"));
        match ready {
            Ok((characteristic, mode, monitor)) => Ok(Self {
                disconnect_monitor: Some(monitor),
                peripheral,
                characteristic,
                mode,
                name,
                usable: true,
            }),
            Err(reason) => {
                let _ = timeout(Duration::from_secs(3), peripheral.disconnect()).await;
                Err(reason.into())
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn take_disconnect_monitor(&mut self) -> Option<crate::ble_connection::DisconnectMonitor> {
        self.disconnect_monitor.take()
    }

    pub async fn is_connected(&self) -> bool {
        self.usable && crate::ble_connection::connection_state(&self.peripheral).await == Some(true)
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        timeout(Duration::from_secs(4), self.peripheral.disconnect())
            .await
            .map_err(|_| "P2 断开超时。".to_owned())?
            .map_err(|_| "无法断开 P2，请检查系统蓝牙连接。".into())
    }

    async fn write(&self, message: &[u8]) -> Result<(), String> {
        timeout(
            Duration::from_secs(5),
            self.peripheral
                .write(&self.characteristic, message, self.mode),
        )
        .await
        .map_err(|_| "P2 写入超时。")?
        .map_err(|_| "无法向 P2 写入打印数据。")?;
        if matches!(self.mode, WriteType::WithoutResponse) {
            sleep(Duration::from_millis(5)).await;
        }
        Ok(())
    }

    pub async fn print_page(&mut self, pixels: &[u8], density: u8) -> Result<(), String> {
        let stream = ble_page::encode(pixels, density)?;
        if !self.is_connected().await {
            return Err("P2 已断开，请重新连接。".into());
        }
        // Both single-PDU ATT write modes have a three-byte opcode/handle header.
        let capacity = usize::from(self.peripheral.mtu())
            .checked_sub(3)
            .filter(|&n| n > 0)
            .ok_or("P2 蓝牙写入容量无效。")?
            .min(512);
        let started = std::time::Instant::now();
        let send = async {
            for packet in stream.chunks(capacity) {
                self.write(packet).await?;
            }
            Ok(())
        };
        let sent = timeout(Duration::from_secs(90), send)
            .await
            .unwrap_or_else(|_| Err("P2 整张发送超时。".into()));
        if sent.is_err() {
            // A partial page must not be resumed or followed by a new job on this stream.
            self.usable = false;
            let _ = self.disconnect().await;
        } else {
            if std::env::var_os("LABEL_PRINTER_BLE_TRACE").is_some() {
                eprintln!(
                    "P2 BLE {:?}: {} bytes, {} bytes/write, {} writes, {:.1} ms",
                    self.mode,
                    stream.len(),
                    capacity,
                    stream.len().div_ceil(capacity),
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            sleep(Duration::from_secs(2)).await;
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_levels_encode_the_default_and_both_limits() {
        for (level, value) in [(DEFAULT_DENSITY, 0), (9, 8), (15, 14)] {
            assert_eq!(
                density_command(level).unwrap(),
                [0x1f, 0x43, 1, value, 0x88]
            );
        }
        assert!(density_command(0).is_err());
        assert!(density_command(16).is_err());
    }

    #[test]
    fn p2_name_excludes_other_models_using_same_service() {
        for name in ["P2", "P2-123", "P2_A"] {
            assert!(is_p2(name));
        }
        for name in ["P20", "P2P-123", "P1", "DP27P", "P2S"] {
            assert!(!is_p2(name));
        }
    }

    #[test]
    fn ble_batches_the_captured_solid_page_and_preserves_its_rows() {
        let mut row = [0; STRIDE];
        row[4..44].fill(0xff);
        let density = density_command(1).unwrap();
        let stream = ble_page::encode(&row.repeat(240), 1).unwrap();
        let expected = [
            SETUP.concat(),
            density.to_vec(),
            vec![0x1f, 0x2b, 4, 40],
            vec![0xff; 40],
            vec![0x1f, 0x2e, 0xc0, 0xee, 0x0c],
        ]
        .concat();
        assert_eq!(stream, expected);
    }

    #[test]
    fn ble_repeat_count_boundaries_and_blank_runs() {
        let density = density_command(1).unwrap();
        for (count, repeat) in [
            (1, vec![]),
            (2, vec![0]),
            (193, vec![191]),
            (194, vec![0xc0, 192]),
            (258, vec![0xc1, 0]),
        ] {
            let mut row = [0; STRIDE];
            row[2] = 1;
            let mut page = row.repeat(count);
            page.extend([0; STRIDE]);
            page.extend(row);
            let stream = ble_page::encode(&page, 1).unwrap();
            let mut expected = vec![0x1f, 0x2b, 2, 1, 1];
            if !repeat.is_empty() {
                expected.extend([0x1f, 0x2e]);
                expected.extend(repeat);
            }
            expected.extend([0x1b, 0x4a, 1, 0x1f, 0x2b, 2, 1, 1, 0x0c]);
            assert_eq!(
                &stream[SETUP.iter().map(|s| s.len()).sum::<usize>() + density.len()..],
                expected
            );
        }
    }

    #[test]
    fn sparse_rows_preserve_head_position_and_feed_length() {
        let mut page = vec![0; STRIDE * 260];
        page[256 * STRIDE + 3] = 0x81;
        page[256 * STRIDE + 5] = 0x40;
        let encoded = bitmap_messages(&page).unwrap();
        assert_eq!(
            encoded,
            [
                vec![0x1b, 0x4a, 255],
                vec![0x1b, 0x4a, 1],
                vec![0x1f, 0x2b, 3, 3, 0x81, 0, 0x40],
                vec![0x1b, 0x4a, 3, 0x0c],
            ]
        );
        let full = vec![0xff; STRIDE];
        assert_eq!(
            bitmap_messages(&full).unwrap(),
            [[&[0x1f, 0x2b, 0, 48][..], full.as_slice(), &[0x0c]].concat()]
        );
        assert_eq!(
            bitmap_messages(&[0; STRIDE]).unwrap(),
            [vec![0x1b, 0x4a, 1, 0x0c]]
        );
    }

    #[test]
    fn bitmap_rows_keep_dot_order_and_finalize_only_the_last_message() {
        let mut pixels = [0u8; 2 * STRIDE];
        pixels[0] = 0x80;
        pixels[pixels.len() - 1] = 0x01;
        let messages: Vec<_> = bitmap_messages(&pixels).unwrap();
        assert_eq!(SETUP[1], [0x1f, 0x27, 1, 48, 0x88]);
        assert_eq!(messages[0], [0x1f, 0x2b, 0, 1, 0x80]);

        assert_eq!(messages[1], [0x1f, 0x2b, 47, 1, 0x01, 0x0c]);
        assert_eq!(messages.len(), 2);
        assert!(bitmap_messages(&[]).is_err());
        assert!(bitmap_messages(&[0; STRIDE + 1]).is_err());
        assert!(bitmap_messages(&vec![0; STRIDE * (MAX_ROWS + 1)]).is_err());
        assert_eq!(
            bitmap_messages(&vec![0; STRIDE * MAX_ROWS]).unwrap().len(),
            MAX_ROWS.div_ceil(255)
        );
    }
}
