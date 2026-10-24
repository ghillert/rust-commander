/// The application name used in windows, menus, and help.
pub const APP_NAME: &str = "Rust Commander";

pub mod app;
pub mod appearance;
pub mod archive;
pub mod background;
pub mod comparison;
pub mod desktop;
pub mod filesystem;
pub mod folder_tools;
pub mod model;
#[cfg(target_os = "macos")]
mod native_menu;
pub mod operations;
pub mod path_rules;
pub mod preferences;
pub mod remote;
pub mod search;
pub mod settings;
pub mod terminal;
pub mod tools_ui;
pub mod transfer;
pub mod transfer_control;
pub mod uri;
mod uri_backend;
pub mod viewer;

pub mod archive_tools;
pub mod feature_ui;
pub mod history;
pub mod i18n;
pub mod icons;
pub mod native_files;
mod native_icons;
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

pub mod navigation;

pub mod config_import;
pub mod text_document;

pub mod file_metadata;

pub mod directory_tree;

pub mod viewer_search;

pub mod associations;
pub mod external_edit;

pub mod storage;

pub mod resumable;

mod remote_upload;

pub mod drag_drop;

#[cfg(target_os = "linux")]
mod native_drag;
