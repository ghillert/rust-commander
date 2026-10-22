//! Native Linux file drags on the existing Slint/winit surface (no helper window).
#[cfg(target_os = "linux")]
mod x11;
use anyhow::Result;
use std::path::PathBuf;

pub enum Event {
    Hover(f32, f32),
    Leave,
    Drop(f32, f32, Vec<PathBuf>),
}
pub struct Linux {
    backend: Backend,
    // The foreign Wayland connection/surface must outlive the worker.
    _window: slint::WindowHandle,
}
enum Backend {
    Wayland {
        device: rust_commander_wayland_dnd::FileDnd,
        finished: Option<Box<dyn Fn() + Send>>,
    },
    X11(Box<x11::Pointer>),
}
impl Linux {
    pub fn attach(window: slint::WindowHandle, wake: impl Fn() + Send + 'static) -> Result<Self> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let backend = match window.window_handle()?.as_raw() {
            RawWindowHandle::Wayland(_) => Backend::Wayland {
                device: rust_commander_wayland_dnd::FileDnd::attach(&window, wake)
                    .ok_or_else(|| anyhow::anyhow!("Could not attach the Wayland drag adapter"))?,
                finished: None,
            },
            RawWindowHandle::Xlib(handle) => {
                Backend::X11(Box::new(x11::Pointer::new(u32::try_from(handle.window)?)?))
            }
            RawWindowHandle::Xcb(handle) => {
                Backend::X11(Box::new(x11::Pointer::new(handle.window.get())?))
            }
            _ => anyhow::bail!("Unsupported Linux window handle"),
        };
        Ok(Self {
            backend,
            _window: window,
        })
    }
    pub fn poll(&mut self) -> Vec<Event> {
        match &mut self.backend {
            Backend::Wayland { device, finished } => device
                .poll()
                .into_iter()
                .filter_map(|event| match event {
                    rust_commander_wayland_dnd::DndEvent::Hovered { x, y } => {
                        Some(Event::Hover(x, y))
                    }
                    rust_commander_wayland_dnd::DndEvent::Left => Some(Event::Leave),
                    rust_commander_wayland_dnd::DndEvent::Dropped { x, y, paths } => {
                        Some(Event::Drop(x, y, paths))
                    }
                    rust_commander_wayland_dnd::DndEvent::DragEnded => {
                        if let Some(callback) = finished.take() {
                            callback();
                        }
                        None
                    }
                })
                .collect(),
            Backend::X11(_) => Vec::new(),
        }
    }
    pub fn pointer(&self, scale: f64) -> Option<(f32, f32)> {
        match &self.backend {
            Backend::X11(pointer) => pointer.position(scale),
            _ => None,
        }
    }
    pub fn pressed(&self) -> bool {
        match &self.backend {
            Backend::Wayland { device, .. } => device.left_pressed(),
            Backend::X11(pointer) => pointer.buttons().0,
        }
    }
    pub fn start(
        &mut self,
        paths: Vec<PathBuf>,
        finished: impl Fn() + Send + 'static,
    ) -> Result<()> {
        match &mut self.backend {
            Backend::Wayland {
                device,
                finished: callback,
            } => {
                *callback = Some(Box::new(finished));
                device.start_drag(paths, false);
                Ok(())
            }
            Backend::X11(_) => x11::start(paths, finished),
        }
    }
}
