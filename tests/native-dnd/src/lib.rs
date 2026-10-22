//! Protocol fixtures compile the application's actual XDND source with a tiny dependency graph.
#[cfg(all(target_os = "linux", test))]
#[path = "../../../src/native_drag/x11.rs"]
mod x11;
