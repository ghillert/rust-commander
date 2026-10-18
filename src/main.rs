#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use rust_commander::{APP_NAME, app::Commander};
use std::path::PathBuf;

fn main() -> eframe::Result {
    #[cfg(target_os = "linux")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == "--drag-helper")
    {
        std::process::exit(rust_commander::linux_drag::run(
            std::env::args_os().skip(2).map(PathBuf::from).collect(),
        ));
    }
    let mut paths = Vec::new();
    for arg in std::env::args_os().skip(1) {
        if arg == "--help" || arg == "-h" {
            println!(
                "{APP_NAME}\n\nUsage: rust-commander [LEFT_DIRECTORY] [RIGHT_DIRECTORY]\n\nNative dual-pane file manager. F1 opens keyboard help."
            );
            return Ok(());
        }
        if arg == "--version" {
            println!("{APP_NAME} {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        paths.push(PathBuf::from(arg));
    }
    if paths.len() > 2 {
        eprintln!("Expected at most two starting directories. Use --help for usage.");
        std::process::exit(2);
    }
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1240.0, 780.0])
        .with_min_inner_size([840.0, 480.0]);
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(true);
    eframe::run_native(
        APP_NAME,
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(Commander::new(cc, &paths)))),
    )
}
