/// The application name used in windows, menus, and help.
pub const APP_NAME: &str = "Rust Commander";

pub mod app;
pub mod appearance;
pub mod archive;
pub mod background;
pub mod desktop;
pub mod filesystem;
pub mod model;
#[cfg(target_os = "macos")]
mod native_menu;
pub mod operations;
pub mod preferences;
pub mod remote;
pub mod settings;
pub mod terminal;
pub mod tools_ui;
pub mod transfer;
pub mod viewer;

pub mod archive_tools;
pub mod feature_ui;
pub mod history;
pub mod i18n;
pub mod native_files;
pub mod oauth;

pub mod native_actions;
pub mod queue;
pub mod sync;
pub mod workspaces;

pub mod smb;

pub mod next_ui;
pub mod preview;

#[cfg(target_os = "linux")]
pub mod linux_drag;

pub mod ui_signal;
