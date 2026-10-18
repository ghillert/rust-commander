// Copyright 2023-2023 CrabNebula Ltd.
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Native macOS and Windows drag-out support for Rust Commander.
//!
//! Adapted from drag 2.1.1. See README.md for upstream provenance and changes.
//! Linux uses Rust Commander's separate GTK4 helper.

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
compile_error!(
    "This vendored drag crate supports macOS and Windows only; use the GTK4 helper on Linux."
);

use std::path::PathBuf;

mod platform_impl;
pub use platform_impl::start_drag;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(windows)]
    #[error("{0}")]
    WindowsError(#[from] windows::core::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("unsupported window handle")]
    UnsupportedWindowHandle,
    #[error("failed to start drag")]
    FailedToStartDrag,
    #[error("drag image not found")]
    ImageNotFound,
    #[error("failed to drop items")]
    FailedToDrop,
    #[error("failed to get cursor position")]
    FailedToGetCursorPosition,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum DragResult {
    Dropped,
    Cancel,
}

pub type DataProvider = Box<dyn Fn(&str) -> Option<Vec<u8>>>;

/// Item to be dragged.
pub enum DragItem {
    /// A list of files to be dragged.
    ///
    /// The paths must be absolute.
    Files(Vec<PathBuf>),
    /// Data to share with another app.
    ///
    /// - **Windows**: Not supported. Will result in a dummy drag operation of current folder that will be cancelled upon dropping.
    Data {
        provider: DataProvider,
        types: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(u64)]
pub enum DragMode {
    #[default]
    Copy = 1, // NSDragOperationCopy
    Move = 16, // NSDragOperationMove
}

#[cfg(target_os = "macos")]
impl From<DragMode> for objc2_app_kit::NSDragOperation {
    fn from(value: DragMode) -> Self {
        match value {
            DragMode::Copy => objc2_app_kit::NSDragOperation::Copy,
            DragMode::Move => objc2_app_kit::NSDragOperation::Move,
        }
    }
}

#[derive(Default)]
pub struct Options {
    // TODO: Fix typo in v3
    pub skip_animatation_on_cancel_or_failure: bool,
    pub mode: DragMode,
}

/// An image definition.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Image {
    /// A path to a image.
    File(PathBuf),
    /// Raw bytes of the image.
    Raw(Vec<u8>),
}

/// Logical position of the cursor.
///
/// - **Windows**: Currently the win32 API for logical position reports physical position as well, due to the complicated nature of potential multiple monitor with different scaling there's no trivial solution to be incorporated.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct CursorPosition {
    pub x: i32,
    pub y: i32,
}
