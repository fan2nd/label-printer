// DeTong USB transport: one numbered HID output report holds up to 62 stream bytes.
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

const ID: u8 = 0x1e;
const MAX_PAYLOAD: usize = 62;

fn report(payload: &[u8]) -> [u8; 64] {
    let mut r = [0; 64];
    r[0] = ID;
    r[1] = payload.len() as u8;
    r[2..2 + payload.len()].copy_from_slice(payload);
    r
}

fn write(device: &HidDevice, data: &[u8]) -> Result<(), String> {
    for chunk in data.chunks(MAX_PAYLOAD) {
        let n = device
            .write(&report(chunk))
            .map_err(|e| format!("P2 USB 写入失败：{e}"))?;
        if n != 64 {
            return Err("P2 USB 报告写入不完整。".into());
        }
    }
    Ok(())
}

fn query(device: &HidDevice, cmd: u8, args: &[u8]) -> Result<Vec<u8>, String> {
    let mut q = vec![0x1f, cmd, args.len() as u8];
    q.extend_from_slice(args);
    q.push(!q[1..].iter().fold(0u8, |s, b| s.wrapping_add(*b)));
    write(device, &q)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let mut r = [0; 64];
        let n = device
            .read_timeout(&mut r, 200)
            .map_err(|e| format!("P2 USB 读取失败：{e}"))?;
        if n < 6 || r[0] != ID || usize::from(r[1]) + 2 > n {
            continue;
        }
        let packet = &r[2..2 + usize::from(r[1])];
        if packet.len() < 4
            || packet[..2] != [0x1f, cmd]
            || usize::from(packet[2]) + 4 != packet.len()
        {
            continue;
        }
        if packet[1..].iter().fold(0u8, |s, b| s.wrapping_add(*b)) != 255
            && packet.last() != Some(&0x88)
        {
            continue;
        }
        return Ok(packet[3..packet.len() - 1].to_vec());
    }
    Err("P2 未回复 USB 查询，请检查数据线、设备状态和端口占用。".into())
}

fn budget(device: &HidDevice) -> Result<usize, String> {
    let data = query(device, 0x77, &[0, 0])?;
    let Some((&first, rest)) = data.split_first() else {
        return Err("P2 缓冲区回包为空。".into());
    };
    let (units, status) = if first < 0xc0 {
        (usize::from(first), rest)
    } else {
        let (&lo, status) = rest.split_first().ok_or("P2 缓冲区回包不完整。")?;
        ((usize::from(first & 0x3f) << 8) | usize::from(lo), status)
    };
    if status.len() < 6 {
        return Err("P2 缓冲区状态不完整。".into());
    }
    if status[..4] != [0; 4] && status[..4] != [0, 0, 0, 4] {
        return Err("P2 未就绪，请检查耗材和仓盖。".into());
    }
    Ok(if units == 1 { 1000 } else { units * 500 })
}

pub struct Printer {
    name: String,
    device: Option<HidDevice>,
}

impl Printer {
    pub async fn connect() -> Result<Self, String> {
        tokio::task::spawn_blocking(|| {
            let api = HidApi::new().map_err(|e| format!("无法启用 USB HID：{e}"))?;
            let mut found = None;
            for info in api
                .device_list()
                .filter(|i| i.vendor_id() == 0x3533 && i.product_id() == 0x5a11)
            {
                let device = info
                    .open_device(&api)
                    .map_err(|e| format!("无法打开德佟 USB HID：{e}"))?;
                let body = query(&device, 0x79, &[])?;
                let name = std::str::from_utf8(body.strip_suffix(&[0]).unwrap_or(&body))
                    .map_err(|_| "无法读取 P2 设备名。")?;
                if !super::is_p2(name) {
                    continue;
                }
                if found.is_some() {
                    return Err("检测到多台 USB P2，请只连接目标打印机。".into());
                }
                found = Some(Self {
                    name: name.to_owned(),
                    device: Some(device),
                });
            }
            found.ok_or_else(|| "未找到 USB P2，请检查打印机电源和 USB 数据线。".into())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn connected(&self) -> bool {
        self.device.is_some()
    }
    pub fn disconnect(&mut self) {
        self.device.take();
    }
    pub async fn print_page(&mut self, page: &[u8]) -> Result<(), String> {
        let rows = super::bitmap_messages(page)?;
        let messages: Vec<_> = super::SETUP
            .iter()
            .map(|m| m.to_vec())
            .chain(rows)
            .collect();
        let device = self.device.take().ok_or("P2 USB 已断开。")?;
        self.device = Some(
            tokio::task::spawn_blocking(move || -> Result<_, String> {
                let mut available = 0;
                for message in messages {
                    let deadline = Instant::now() + Duration::from_secs(30);
                    while available < message.len() {
                        available = budget(&device)?;
                        if available < message.len() {
                            if Instant::now() >= deadline {
                                return Err("P2 USB 缓冲区持续忙碌。".into());
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    }
                    write(&device, &message)?;
                    available -= message.len();
                }
                Ok(device)
            })
            .await
            .map_err(|e| e.to_string())??,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hid_report_carries_only_declared_bytes() {
        let r = super::report(&[0x1f, 0x79, 0, 0x86]);
        assert_eq!(&r[..6], &[0x1e, 4, 0x1f, 0x79, 0, 0x86]);
        assert!(r[6..].iter().all(|b| *b == 0));
    }
}
