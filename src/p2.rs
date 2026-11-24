//! DeTong P2 BLE printing. The wire format is described in IvanLi-CN/detonger
//! (docs/ble.md and docs/protocol.md). A successful write is a submission only.
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
const STRIDE: usize = 48;
const MAX_ROWS: usize = 1600;

// One die-cut label per job. Each entry is a separate, short GATT message.
const SETUP: [&[u8]; 5] = [
    &[0x1f, 0x20, 2, 0, 1, 0x88], // start page 1
    &[0x1f, 0x27, 1, STRIDE as u8, 0x88],
    &[0x1f, 0x42, 1, 2, 0x88],          // gap media
    &[0x1f, 0x45, 2, 0xff, 0xff, 0x88], // automatic gap length
    &[0x1f, 0x43, 1, 8, 0x88],          // darkness
];

fn bitmap_messages(pixels: &[u8]) -> Result<impl Iterator<Item = Vec<u8>> + '_, String> {
    if pixels.is_empty() || pixels.len() % STRIDE != 0 || pixels.len() > STRIDE * MAX_ROWS {
        return Err("P2 图像尺寸无效，请检查标签规格。".into());
    }
    let rows = pixels.chunks_exact(STRIDE);
    let height = rows.len();
    Ok(rows.enumerate().map(move |(index, row)| {
        let mut message = Vec::with_capacity(4 + STRIDE + 1);
        message.extend_from_slice(&[0x1f, 0x2b, 0, STRIDE as u8]);
        message.extend_from_slice(row);
        if index + 1 == height {
            message.push(0x0c);
        }
        message
    }))
}

fn is_p2(name: &str) -> bool {
    matches!(name.split(['-', '_']).next(), Some("P2"))
}

pub struct Printer {
    _manager: Manager,
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
            sleep(Duration::from_secs(4)).await;
            let devices = timeout(Duration::from_secs(3), adapter.peripherals()).await;
            let _ = timeout(Duration::from_secs(2), adapter.stop_scan()).await;
            let Ok(Ok(devices)) = devices else { continue };
            for peripheral in devices {
                if let Ok(Ok(Some(props))) =
                    timeout(Duration::from_secs(2), peripheral.properties()).await
                {
                    if let Some(name) = props.local_name.filter(|name| is_p2(name)) {
                        found.push((peripheral, name));
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
        let (peripheral, name) = found
            .pop()
            .ok_or("未找到 P2，请开启打印机并断开其他应用的连接。")?;
        let setup = async {
            peripheral.connect().await.map_err(|_| "无法连接 P2。")?;
            peripheral
                .discover_services()
                .await
                .map_err(|_| "无法读取 P2 蓝牙服务。")?;
            let characteristic = peripheral
                .characteristics()
                .into_iter()
                .find(|c| c.service_uuid == SERVICE && c.uuid == WRITE)
                .ok_or("所选设备没有 P2 打印服务。")?;
            let flags = characteristic.properties;
            let mode = if flags.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
                WriteType::WithoutResponse
            } else if flags.contains(CharPropFlags::WRITE) {
                WriteType::WithResponse
            } else {
                return Err("P2 打印服务不可写入。");
            };
            Ok((characteristic, mode))
        };
        let ready = timeout(Duration::from_secs(16), setup)
            .await
            .unwrap_or(Err("连接 P2 超时，请重新连接。"));
        match ready {
            Ok((characteristic, mode)) => Ok(Self {
                _manager: manager,
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

    pub async fn is_connected(&self) -> bool {
        self.usable
            && timeout(Duration::from_secs(3), self.peripheral.is_connected())
                .await
                .is_ok_and(|result| result.unwrap_or(false))
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
        sleep(Duration::from_millis(5)).await;
        Ok(())
    }

    pub async fn print_page(&mut self, pixels: &[u8]) -> Result<(), String> {
        let rows = bitmap_messages(pixels)?;
        if !self.is_connected().await {
            return Err("P2 已断开，请重新连接。".into());
        }
        let send = async {
            for command in SETUP {
                self.write(command).await?;
            }
            for row in rows {
                self.write(&row).await?;
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
            sleep(Duration::from_secs(2)).await;
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bitmap_rows_keep_dot_order_and_finalize_only_the_last_message() {
        let mut pixels = [0u8; 2 * STRIDE];
        pixels[0] = 0x80;
        pixels[pixels.len() - 1] = 0x01;
        let messages: Vec<_> = bitmap_messages(&pixels).unwrap().collect();
        assert_eq!(SETUP[1], [0x1f, 0x27, 1, 48, 0x88]);
        assert_eq!(&messages[0][..5], &[0x1f, 0x2b, 0, 48, 0x80]);
        assert_eq!(messages[0].len(), 52);
        assert_eq!(&messages[1][51..], &[0x01, 0x0c]);
        assert_eq!(messages.len(), 2);
        assert!(bitmap_messages(&[]).is_err());
        assert!(bitmap_messages(&[0; STRIDE + 1]).is_err());
        assert!(bitmap_messages(&vec![0; STRIDE * (MAX_ROWS + 1)]).is_err());
        assert_eq!(
            bitmap_messages(&vec![0; STRIDE * MAX_ROWS])
                .unwrap()
                .count(),
            MAX_ROWS
        );
    }
}
