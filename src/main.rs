#![cfg_attr(
    all(windows, feature = "desktop", not(test)),
    windows_subsystem = "windows"
)]

#[cfg(feature = "ble")]
mod ble_connection;
#[cfg(feature = "desktop")]
mod cli;
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
mod table;
#[cfg(feature = "desktop")]
mod ui;
mod workbook;

#[cfg(feature = "desktop")]
fn main() {
    if std::env::args_os().nth(1).is_some_and(|first| {
        !(cfg!(target_os = "macos") && first.to_string_lossy().starts_with("-psn_"))
    }) {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn AttachConsole(process: u32) -> i32;
            }
            unsafe {
                AttachConsole(u32::MAX);
            }
        }
        if let Err(reason) = cli::run() {
            eprintln!("{reason}");
            std::process::exit(1);
        }
        return;
    }
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
