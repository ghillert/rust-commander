//! File drag and drop between a window and other applications.
//!
//! winit does not speak the Wayland data-device protocol, so a window on
//! Wayland neither receives dropped files nor can start a drag. This crate
//! attaches a second event queue to the window's own `wl_display`, the same
//! technique `smithay-clipboard` uses for the clipboard, and owns a data
//! device per seat. The compositor then reports the pointer position on every
//! motion of an incoming drag, and accepts drags started from the window.
//!
//! X11, Windows and macOS are served by winit's own file-drop events;
//! `attach` returns `None` there.
#![warn(missing_docs)]

use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod pipe;
mod uri_list;
#[cfg(target_os = "linux")]
mod wayland;

/// Positions are in logical pixels relative to the window's surface.
#[derive(Clone, Debug, PartialEq)]
pub enum DndEvent {
    /// Files from any application are over the window.
    Hovered {
        /// Pointer position from the left edge of the surface.
        x: f32,
        /// Pointer position from the top edge of the surface.
        y: f32,
    },
    /// The drag left the window without dropping.
    Left,
    /// Files were dropped on the window.
    Dropped {
        /// Last pointer position of the drag, from the left edge of the surface.
        x: f32,
        /// Last pointer position of the drag, from the top edge of the surface.
        y: f32,
        /// The local files of the drop, in the order of the source's URI list. Never empty.
        paths: Vec<PathBuf>,
    },
    /// A drag started with `start_drag` was dropped or cancelled.
    DragEnded {
        /// Whether the compositor reported an accepted drop.
        accepted: bool,
    },
}

/// A window's connection to the Wayland drag-and-drop protocol. Dropping it stops and joins
/// its worker thread.
pub struct FileDnd {
    #[cfg(target_os = "linux")]
    inner: wayland::Worker,
}

impl FileDnd {
    /// Attach to a live window. `wake` runs on a worker thread whenever an
    /// event is ready, typically to request a repaint.
    ///
    /// The window must outlive the returned value.
    pub fn attach(
        window: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
        wake: impl Fn() + Send + 'static,
    ) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
            let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) = (
                window.display_handle().ok()?.as_raw(),
                window.window_handle().ok()?.as_raw(),
            ) else {
                return None;
            };
            // Safety: both pointers come from a live window that the caller
            // keeps alive for the lifetime of the worker.
            let inner = unsafe {
                wayland::Worker::spawn(
                    display.display.as_ptr(),
                    surface.surface.as_ptr(),
                    Box::new(wake),
                )
            }?;
            Some(Self { inner })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (window, wake);
            None
        }
    }

    /// Events since the last call, oldest first.
    pub fn poll(&self) -> Vec<DndEvent> {
        #[cfg(target_os = "linux")]
        return self.inner.events.try_iter().collect();
        #[cfg(not(target_os = "linux"))]
        Vec::new()
    }

    /// Whether the initiating pointer button is still held on the host seat.
    pub fn left_pressed(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.inner.left_pressed();
        #[cfg(not(target_os = "linux"))]
        false
    }

    /// Hand `paths` to the compositor as a drag. Call while the pointer button
    /// that started the gesture is still held; the compositor rejects a drag
    /// without a recent press on this window. From here the compositor owns
    /// the pointer: the window sees it leave, and a drop back onto the window
    /// arrives as `Dropped`.
    pub fn start_drag(&self, paths: Vec<PathBuf>, allow_move: bool) {
        #[cfg(target_os = "linux")]
        self.inner.start_drag(uri_list::encode(&paths), allow_move);
        #[cfg(not(target_os = "linux"))]
        let _ = (paths, allow_move);
    }
}

/// A provider that is called only after the compositor reports a performed drop.
pub type DeferredFiles = std::sync::Arc<dyn Fn() -> Result<Vec<PathBuf>, String> + Send + Sync>;
impl FileDnd {
    /// Offer files without downloading or extracting anything during the gesture.
    pub fn start_deferred(&self, provider: DeferredFiles) {
        #[cfg(target_os = "linux")]
        self.inner.start_deferred(std::sync::Arc::new(move || {
            provider().map(|paths| uri_list::encode(&paths))
        }));
        #[cfg(not(target_os = "linux"))]
        let _ = provider;
    }
}
