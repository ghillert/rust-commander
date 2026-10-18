# Vendored native drag support

This is the macOS and Windows subset of [CrabNebula drag-rs](https://github.com/crabnebula-dev/drag-rs),
from the crates.io `drag` **2.1.1** release, upstream commit
`8b48f9883658507527287169b53d4ec7579b540c` (`crates/drag`).

## Why this copy exists

The published crate unconditionally declares a GTK3 backend. Cargo records its
GLib 0.18 dependency in Cargo.lock even when the application only enables drag
on macOS and Windows. That GLib version is affected by
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html), and
GTK3 cannot resolve to the fixed GLib 0.20+ series. Rust Commander already uses
its own GTK4 helper on Linux, so the GTK3 backend serves no purpose here.

## Changes from upstream

- Removed GTK3/GDK/GDK-X11 dependencies, backend source, and backend selection.
- Removed upstream example/development dependencies and Linux-only API/docs.
- Added an explicit unsupported-platform error and disabled publishing.
- Retained the macOS and Windows backend sources without changes.

The root manifest patches `drag` to this local crate. No GLib advisory is ignored
or suppressed. Linux continues to use `src/linux_drag.rs` and GTK4. Remove this
copy and its root patch when a published upstream release allows excluding GTK3
from the resolved dependency graph; verify Cargo.lock and all three platforms
when updating.

## License

Upstream copyright and both MIT and Apache-2.0 license files are retained.
Rust Commander redistributes this subset under the MIT option, with its notice
included in the root NOTICE so packaged executables carry it too.
