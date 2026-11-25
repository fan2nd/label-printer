// Brother PT-P300BT, 12 mm continuous TZe tape. The transport owns only the
// RFCOMM byte stream; status interpretation and print framing stay in Rust.
use std::{ffi::c_void, ptr::NonNull, time::Duration};
use tokio::time::{sleep, Instant};

extern "C" {
    fn lp_brother_open(count: *mut i32) -> *mut c_void;
    fn lp_brother_state(handle: *mut c_void) -> i32;
    fn lp_brother_write(handle: *mut c_void, data: *const u8, length: i32) -> i32;
    fn lp_brother_status(handle: *mut c_void, frame: *mut u8) -> i32;
    fn lp_brother_close(handle: *mut c_void);
}

pub struct Printer(NonNull<c_void>);

impl Drop for Printer {
    fn drop(&mut self) {
        unsafe { lp_brother_close(self.0.as_ptr()) }
    }
}

fn init() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes.extend_from_slice(b"\x1b@\x1bia\x01");
    bytes
}

fn check_status(frame: &[u8; 32], expected: u8) -> Result<bool, String> {
    if frame[..5] != [0x80, 0x20, 0x42, 0x30, 0x72] {
        return Err("收到的状态与 PT-P300BT 不匹配。".into());
    }
    if frame[8] != 0 || frame[9] != 0 || matches!(frame[18], 2 | 4) {
        return Err(format!(
            "打印机报告异常，请检查电源、色带和仓盖（{:02X}/{:02X}）。",
            frame[8], frame[9]
        ));
    }
    if frame[18] != expected {
        return Ok(false);
    }
    if expected == 0 {
        if frame[10] != 12 || frame[17] != 0 {
            return Err(format!(
                "请装入 12 mm 连续色带，当前回报宽度为 {} mm。",
                frame[10]
            ));
        }
        if frame[19..22] != [0; 3] {
            return Err("打印机正在工作，请稍后再试。".into());
        }
    }
    Ok(true)
}

fn encode(page: &[u8], media_type: u8) -> Result<Vec<u8>, String> {
    if page.is_empty() || page.len() % 16 != 0 {
        return Err("PT-P300BT 点阵尺寸无效。".into());
    }
    let mut job = init();
    job.extend_from_slice(&[0x1b, 0x69, 0x7a, 0xc4, media_type, 12, 0]);
    job.extend_from_slice(&((page.len() / 16) as u32).to_le_bytes());
    job.extend_from_slice(b"\x00\x00\x1biK\x08\x1biM\x00\x1bid\x00\x00M\x00");
    for row in page.chunks_exact(16) {
        if row.iter().all(|&b| b == 0) {
            job.push(0x5a);
        } else {
            job.extend_from_slice(&[0x47, 16, 0]);
            // The shared preview raster numbers head dots from the lower edge.
            // Brother places the uppermost printable dot at byte 4, bit 7.
            job.extend(row.iter().rev().map(|b| b.reverse_bits()));
        }
    }
    job.push(0x1a);
    Ok(job)
}

impl Printer {
    pub async fn connect() -> Result<Self, String> {
        let mut count = 0;
        let raw = unsafe { lp_brother_open(&mut count) };
        let printer = Self(NonNull::new(raw).ok_or(if count > 1 {
            "已配对多台 PT-P300BT，请仅保留目标设备的配对后重试。"
        } else {
            "请先在 macOS 蓝牙设置中配对 PT-P300BT，并允许蓝牙访问。"
        })?);
        let deadline = Instant::now() + Duration::from_secs(18);
        loop {
            match unsafe { lp_brother_state(raw) } {
                1 => break,
                0 if Instant::now() < deadline => sleep(Duration::from_millis(40)).await,
                _ => {
                    return Err(
                        "无法打开 PT-P300BT 蓝牙串口，请检查打印机是否开机或被占用。".into(),
                    )
                }
            }
        }
        printer.write(&init()).await?;
        printer.query().await?;
        Ok(printer)
    }
    pub fn connected(&self) -> bool {
        unsafe { lp_brother_state(self.0.as_ptr()) == 1 }
    }
    async fn write(&self, mut data: &[u8]) -> Result<(), String> {
        while !data.is_empty() {
            let n = unsafe {
                lp_brother_write(self.0.as_ptr(), data.as_ptr(), data.len().min(512) as i32)
            };
            if n <= 0 {
                return Err("蓝牙写入失败，请检查打印机连接。".into());
            }
            data = &data[n as usize..];
            tokio::task::yield_now().await;
        }
        Ok(())
    }
    async fn wait_status(&self, kind: u8, seconds: u64) -> Result<[u8; 32], String> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            let mut frame = [0; 32];
            match unsafe { lp_brother_status(self.0.as_ptr(), frame.as_mut_ptr()) } {
                1 => {
                    if check_status(&frame, kind)? {
                        return Ok(frame);
                    }
                }
                0 => sleep(Duration::from_millis(30)).await,
                _ => return Err("PT-P300BT 连接已断开。".into()),
            }
        }
        Err("等待 PT-P300BT 状态超时。".into())
    }
    async fn query(&self) -> Result<[u8; 32], String> {
        self.write(b"\x1biS").await?;
        self.wait_status(0, 5).await
    }
    pub async fn print_page(&mut self, page: &[u8]) -> Result<(), String> {
        let status = self.query().await?;
        self.write(&encode(page, status[11])?).await?;
        self.wait_status(1, 60).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tape_and_completion_are_verified() {
        let mut frame = [0; 32];
        frame[..5].copy_from_slice(&[128, 32, 66, 48, 114]);
        frame[10] = 9;
        assert!(check_status(&frame, 0).is_err());
        frame[10] = 12;
        assert_eq!(check_status(&frame, 0), Ok(true));
        frame[18] = 6;
        assert_eq!(check_status(&frame, 1), Ok(false));
        frame[18] = 1;
        assert_eq!(check_status(&frame, 1), Ok(true));
        frame[8] = 1;
        assert!(check_status(&frame, 1).is_err());
    }
    #[test]
    fn twelve_mm_canvas_prints_its_top_and_bottom_at_wire_offsets() {
        use crate::{core::Media, device::Model};
        let model = Model::PtP300bt;
        let stock = Media::for_model(model);
        assert_eq!((stock.dpi, stock.canvas_size()), (180, (354, 64)));
        assert!(model.supports(&stock));
        assert!(!Model::P2.supports(&stock));
        let svg="<svg xmlns='http://www.w3.org/2000/svg' width='354' height='64'><rect width='1' height='1'/><rect x='353' y='63' width='1' height='1'/></svg>";
        let page = model.rasterize(svg, &stock).unwrap();
        assert_eq!(page.len(), 354 * 16);
        assert!(model.preview_png(svg, &stock).is_ok());
        let job = encode(&page, 1).unwrap();
        let first = job.windows(3).position(|w| w == [0x47, 16, 0]).unwrap() + 3;
        assert_eq!(job[first + 4], 128);
        assert_eq!(job[job.len() - 1 - 16 + 11], 1);
    }

    #[test]
    fn asymmetric_ink_matches_wire_orientation() {
        let mut page = [0; 32];
        page[16 + 11] = 1;
        let b = encode(&page, 1).unwrap();
        let mut expected = vec![0x5a, 0x47, 16, 0];
        let mut wire = [0; 16];
        wire[4] = 128;
        expected.extend(wire);
        expected.push(0x1a);
        assert!(b.ends_with(&expected));
    }
}
