#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() -> eframe::Result {
    let mut paths = Vec::new();
    for arg in std::env::args_os().skip(1) {
        if arg == "--help" || arg == "-h" {
            println!(
                "muCommander Rust\n\nUsage: mucommander-rust [LEFT_DIRECTORY] [RIGHT_DIRECTORY]\n\nNative dual-pane file manager. F1 opens keyboard help."
            );
            return Ok(());
        }
        if arg == "--version" {
            println!("mucommander-rust {}", env!("CARGO_PKG_VERSION"));
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
        "muCommander Rust",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(mucommander_rust::app::Commander::new(cc, &paths)))),
    )
}
