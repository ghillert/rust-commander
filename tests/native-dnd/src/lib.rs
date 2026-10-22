//! Protocol fixtures compile the application's actual XDND source with a tiny dependency graph.
#[cfg(all(target_os = "linux", test))]
#[path = "../../../src/native_drag/x11.rs"]
mod x11;

#[cfg(all(target_os = "windows", test))]
pub use drag::{
    CursorPosition, DragItem, DragMode, DragResult, DropCallback, DropEvent, Error, Image, Options,
    PromiseEntry, PromiseRead, PromisedFiles, Result,
};
#[cfg(all(target_os = "windows", test))]
#[path = "../../../vendor/drag/src/platform_impl/windows/mod.rs"]
#[allow(dead_code, unused_imports)]
#[rustfmt::skip]
mod windows_provider;
