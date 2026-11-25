use super::{Decoder, Reply};
use serialport::{ClearBuffer, SerialPort, SerialPortType};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub fn ports() -> Result<Vec<(String, String)>, String> {
    let mut ports: Vec<_> = serialport::available_ports()
        .map_err(|e| format!("无法列出 USB 串口：{e}"))?
        .into_iter()
        .filter_map(|port| {
            let SerialPortType::UsbPort(info) = port.port_type else {
                return None;
            };
            #[cfg(target_os = "macos")]
            if port.port_name.starts_with("/dev/tty.") {
                return None;
            }
            let name = info.product.unwrap_or_else(|| "USB 串口".into());
            let label = format!("{name} ({})", port.port_name);
            Some((port.port_name, label))
        })
        .collect();
    ports.sort();
    Ok(ports)
}

pub(super) struct Usb(Arc<Mutex<Box<dyn SerialPort>>>);

impl Usb {
    pub(super) async fn open(path: String) -> Result<Self, String> {
        tokio::task::spawn_blocking(move || {
            serialport::new(path, 115200)
                .timeout(Duration::from_millis(100))
                .open()
                .map(|port| Self(Arc::new(Mutex::new(port))))
                .map_err(|e| format!("无法打开 USB 串口：{e}"))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub(super) fn connected(&self) -> bool {
        self.0.lock().is_ok_and(|port| port.bytes_to_read().is_ok())
    }

    pub(super) async fn send(
        &self,
        frame: Vec<u8>,
        response: Option<(Vec<u8>, Duration)>,
    ) -> Result<Option<Reply>, String> {
        let handle = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut port = handle.lock().map_err(|_| "USB 串口已关闭。")?;
            if response.is_some() {
                port.clear(ClearBuffer::Input)
                    .map_err(|e| format!("USB 串口读取失败：{e}"))?;
            }
            std::thread::sleep(Duration::from_millis(10));
            port.write_all(&frame)
                .map_err(|e| format!("USB 写入失败：{e}"))?;
            let Some((expected, limit)) = response else {
                return Ok(None);
            };
            let deadline = Instant::now() + limit;
            let mut decoder = Decoder::default();
            let mut replies = VecDeque::new();
            let mut bytes = [0; 512];
            while Instant::now() < deadline {
                match port.read(&mut bytes) {
                    Ok(0) => return Err("USB 连接已断开。".into()),
                    Ok(n) => {
                        decoder.push(&bytes[..n], &mut replies);
                        if let Some(reply) =
                            replies.drain(..).find(|r| expected.contains(&r.command))
                        {
                            return Ok(Some(reply));
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                        ) =>
                    {
                        ()
                    }
                    Err(e) => return Err(format!("USB 读取失败：{e}")),
                }
            }
            Err("打印机未在预期时间内响应。".into())
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

#[cfg(all(test, unix))]
#[test]
fn serial_exchange_reads_fragmented_model_reply() {
    let (mut peer, mut port) = serialport::TTYPort::pair().unwrap();
    port.set_timeout(Duration::from_millis(100)).unwrap();
    peer.set_timeout(Duration::from_secs(2)).unwrap();
    let query = super::frame(0x40, &[8]);
    let expected = query.clone();
    let device = std::thread::spawn(move || {
        let mut received = vec![0; expected.len()];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(received, expected);
        let reply = super::frame(0x48, &3586u16.to_be_bytes());
        peer.write_all(&reply[..4]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        peer.write_all(&reply[4..]).unwrap();
        peer
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let usb = Usb(Arc::new(Mutex::new(Box::new(port))));
    let reply = runtime
        .block_on(usb.send(query, Some((vec![0x48], Duration::from_secs(1)))))
        .unwrap()
        .unwrap();
    assert_eq!(super::model_id(&reply), Some(3586));
    device.join().unwrap();
}
