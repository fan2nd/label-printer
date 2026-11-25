//! Device-specific operations used by the desktop print workflow.
use crate::{core::Media, p2, printer, raster};

pub use crate::stock::Model;

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Self::N1 => "N1",
            Self::P2 => "P2",
            Self::PtP300bt => "PT-P300BT",
        }
    }

    pub fn supported_stock(self) -> &'static str {
        match self {
            Self::PtP300bt => "12 mm TZe 连续色带",
            Self::N1 => "12 × 25 mm 标签",
            Self::P2 => "纸宽至多 57 mm、可印宽至多 384 点的间隙标签",
        }
    }

    pub fn supports(self, media: &Media) -> bool {
        match self {
            Self::N1 => media.is_known_n1(),
            Self::P2 => media.supports_p2(),
            Self::PtP300bt => {
                media.dpi == 180
                    && media.width_mm == 12
                    && media.printable_width_dots == 64
                    && media.printable_length_dots == self.length_dots(media.length_mm)
                    && media.validate().is_ok()
            }
        }
    }

    pub fn rasterize(self, svg: &str, media: &Media) -> Result<Vec<u8>, String> {
        if !self.supports(media) {
            return Err(format!("请为 {} 选择支持的标签规格。", self.name()));
        }
        match self {
            Self::N1 => raster::packed_n1(svg, media),
            Self::P2 => raster::packed_p2(svg, media),
            Self::PtP300bt => raster::packed_brother(svg, media),
        }
    }

    /// Subtract this offset from layout X to locate the same dot on the display.
    pub fn preview_feed_shift(self, media: &Media) -> u32 {
        if self == Self::N1 && media.is_known_n1() {
            raster::N1_FEED_SHIFT
        } else {
            0
        }
    }

    pub fn preview_png(self, svg: &str, media: &Media) -> Result<String, String> {
        if !self.supports(media) {
            return raster::stock_preview_png(svg, media);
        }
        let packed = self.rasterize(svg, media)?;
        raster::preview_png(&packed, media, self.head_dots())
    }

    pub async fn connect(self, usb_port: Option<String>) -> Result<Device, String> {
        match (self, usb_port) {
            (Self::N1, Some(port)) => printer::Printer::connect_usb(port).await.map(Device::N1),
            (Self::N1, None) => printer::Printer::connect().await.map(Device::N1),
            #[cfg(target_os = "macos")]
            (Self::PtP300bt, _) => crate::brother::Printer::connect()
                .await
                .map(|p| Device::Brother(Some(p))),
            #[cfg(not(target_os = "macos"))]
            (Self::PtP300bt, _) => Err("PT-P300BT 当前仅支持 macOS。".into()),
            (Self::P2, _) => p2::Printer::connect().await.map(Device::P2),
        }
    }
}

pub enum Device {
    N1(printer::Printer),
    P2(p2::Printer),
    #[cfg(target_os = "macos")]
    Brother(Option<crate::brother::Printer>),
}

impl Device {
    pub fn name(&self) -> &str {
        match self {
            #[cfg(target_os = "macos")]
            Self::Brother(_) => "Brother PT-P300BT",
            Self::N1(device) => device.name(),
            Self::P2(device) => device.name(),
        }
    }

    pub async fn is_connected(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            Self::Brother(p) => p.as_ref().is_some_and(|p| p.connected()),
            Self::N1(device) => device.is_connected().await,
            Self::P2(device) => device.is_connected().await,
        }
    }

    pub async fn print_page(&mut self, page: &[u8]) -> Result<(), String> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Brother(slot) => {
                let result = slot
                    .as_mut()
                    .ok_or("打印机已断开。")?
                    .print_page(page)
                    .await;
                if result.is_err() {
                    slot.take();
                }
                result
            }
            Self::N1(device) => device.print_page(page).await,
            Self::P2(device) => device.print_page(page).await,
        }
    }

    pub async fn disconnect(&mut self) -> Result<(), String> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Brother(slot) => {
                slot.take();
                Ok(())
            }
            Self::N1(device) => device.disconnect().await,
            Self::P2(device) => device.disconnect().await,
        }
    }
}
