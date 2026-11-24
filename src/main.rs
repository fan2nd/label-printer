#![cfg_attr(
    all(windows, feature = "desktop", not(test)),
    windows_subsystem = "windows"
)]

mod core;
#[cfg(feature = "desktop")]
mod device;
mod editor;
#[cfg(feature = "ble")]
mod p2;
mod picture;
#[cfg(any(feature = "desktop", test))]
mod print_job;
#[cfg(feature = "ble")]
mod printer;
mod raster;
mod stock;
mod storage;
mod symbol;
#[cfg(feature = "desktop")]
mod ui;

#[cfg(feature = "desktop")]
fn main() {
    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new().with_window(
                WindowBuilder::new()
                    .with_title(env!("CARGO_PKG_NAME"))
                    .with_maximized(true)
                    .with_inner_size(LogicalSize::new(1240.0, 830.0))
                    .with_min_inner_size(LogicalSize::new(520.0, 560.0)),
            ),
        )
        .launch(ui::App);
}
#[cfg(not(feature = "desktop"))]
fn main() {
    eprintln!("Rebuild with the desktop feature enabled.");
}
