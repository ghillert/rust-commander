//! Rust Commander is a dual-pane file manager for local files, remote connections,
//! and archives. This crate provides the desktop application and the file-management
//! APIs it uses. File operations can also run directly from Rust code.
//!
//! # Working with files
//!
//! - Browse a [`filesystem::Location`] with [`filesystem::list`]. The [`remote`]
//!   module manages the connections used by remote locations.
//! - Use [`model::Tab`] to track navigation and select entries. Open a selected
//!   file with [`viewer::load`] or browse an archive with [`archive::list`].
//! - Use an [`Operation`](operations::Operation) and [`operations::run`] to act on a
//!   selection. A [`Task`](operations::Task) carries progress and cancellation.
//!   [`queue::Queue`] persists pending work across restarts.
//! - Use [`transfer`] to preview local copy or move conflicts and choose how to
//!   handle them. [`resumable`] provides checkpointed copies for local and remote files.
//! - Use [`history::execute`] to journal an operation for Undo. Use [`recovery`]
//!   to retrieve an earlier version as a separate copy.
//!
//! # Examples
//!
//! Copy a selected file into an existing directory using the same operation as
//! the desktop application.
//!
//! ```
//! use rust_commander::operations::{self, Operation, Task};
//! use std::fs;
//!
//! let workspace = tempfile::tempdir()?;
//! let source = workspace.path().join("notes.txt");
//! let destination = workspace.path().join("copies");
//! fs::write(&source, "meeting notes")?;
//! fs::create_dir(&destination)?;
//!
//! let copy = Operation::Copy {
//!     sources: vec![source.clone()],
//!     destination: destination.clone(),
//! };
//! operations::run(copy, &mut Task::default())?;
//!
//! assert_eq!(fs::read_to_string(destination.join("notes.txt"))?, "meeting notes");
//! assert_eq!(fs::read_to_string(source)?, "meeting notes");
//! # Ok::<(), anyhow::Error>(())
//! ```

/// The application name used in windows, menus, and help.
pub const APP_NAME: &str = "Rust Commander";

pub mod app;
pub mod appearance;
pub mod archive;
pub mod background;
pub mod comparison;
pub mod debug_log;
pub mod desktop;
mod discovery;
pub mod filesystem;
pub mod folder_tools;
pub mod mail;
pub mod metadata;
pub mod model;
#[cfg(target_os = "macos")]
mod native_menu;
pub mod open_as;
pub mod operations;
pub mod packing;
pub mod path_rules;
pub mod pdf_text;
pub mod preferences;
pub mod remote;
pub mod search;
pub mod settings;
mod source_view;
pub mod terminal;
pub mod tools_ui;
pub mod transfer;
pub mod transfer_control;
pub mod uri;
mod uri_backend;
pub mod viewer;

pub mod archive_tools;
mod archive_writeback;
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

pub mod recovery;
pub mod storage;
pub mod support;

pub mod resumable;

mod remote_upload;

pub mod drag_drop;

#[cfg(target_os = "linux")]
mod native_drag;

pub mod ssh;

pub mod security_metadata;

pub mod private_bytes;

pub mod editor_drafts;

pub mod high_contrast;

#[cfg(target_os = "windows")]
mod windows_menu;
