//! Device-specific operations used by the desktop print workflow.
use crate::{core::Media, p2, printer, raster};

pub use crate::stock::Model;

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Self::N1 => "N1",
            Self::P2 => "P2",
        }
    }

    pub fn supported_stock(self) -> &'static str {
        match self {
            Self::N1 => "12 × 25 mm 标签",
            Self::P2 => "纸宽至多 57 mm、可印宽至多 384 点的间隙标签",
        }
    }

    pub fn supports(self, media: &Media) -> bool {
        match self {
            Self::N1 => media.is_known_n1(),
            Self::P2 => media.supports_p2(),
        }
    }

    pub fn rasterize(self, svg: &str, media: &Media) -> Result<Vec<u8>, String> {
        if !self.supports(media) {
            return Err(format!("请为 {} 选择支持的标签规格。", self.name()));
        }
        match self {
            Self::N1 => raster::packed_n1(svg, media),
            Self::P2 => raster::packed_p2(svg, media),
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

    pub async fn connect(self, use_usb: bool, usb_port: Option<String>) -> Result<Device, String> {
        match (self, usb_port) {
            (Self::N1, Some(port)) => printer::Printer::connect_usb(port).await.map(Device::N1),
            (Self::N1, None) => printer::Printer::connect().await.map(Device::N1),
            (Self::P2, _) if use_usb => p2::usb::Printer::connect().await.map(Device::P2Usb),
            (Self::P2, _) => p2::Printer::connect().await.map(Device::P2),
        }
    }
}

pub enum Device {
    N1(printer::Printer),
    P2(p2::Printer),
    P2Usb(p2::usb::Printer),
}

impl Device {
    pub fn name(&self) -> &str {
        match self {
            Self::N1(device) => device.name(),
            Self::P2Usb(p) => p.name(),
            Self::P2(device) => device.name(),
        }
    }

    pub fn take_disconnect_monitor(&mut self) -> Option<crate::ble_connection::DisconnectMonitor> {
        match self {
            Self::N1(printer) => printer.take_disconnect_monitor(),
            Self::P2(printer) => printer.take_disconnect_monitor(),
            Self::P2Usb(_) => None,
        }
    }

    pub async fn is_connected(&self) -> bool {
        match self {
            Self::N1(device) => device.is_connected().await,
            Self::P2Usb(p) => p.connected(),
            Self::P2(device) => device.is_connected().await,
        }
    }

    pub async fn print_page(&mut self, page: &[u8]) -> Result<(), String> {
        match self {
            Self::N1(device) => device.print_page(page).await,
            Self::P2Usb(p) => p.print_page(page).await,
            Self::P2(device) => device.print_page(page).await,
        }
    }

    pub async fn disconnect(&mut self) -> Result<(), String> {
        match self {
            Self::N1(device) => device.disconnect().await,
            Self::P2Usb(p) => {
                p.disconnect();
                Ok(())
            }
            Self::P2(device) => device.disconnect().await,
        }
    }
}
