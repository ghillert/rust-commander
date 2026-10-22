# Drag and drop

Drag one file, a folder, or a marked selection to another pane, a folder row,
open tab, breadcrumb, sidebar location, directory-tree node, or writable local
archive. The highlighted target identifies the destination. Dropping opens a
transfer confirmation: choose **Copy** or **Move**, review the destination, and
choose skip, keep both, replace, folder merging, or individual conflict rules.
Cancel leaves both sides unchanged.

Hover for approximately 650 ms to open a folder/location, switch tabs, or expand
a tree node. Hold the pointer near the top or bottom of a file list to scroll.
The original selection stays attached to the gesture when navigation changes the
source pane. Locked tabs follow the normal rule of opening another tab.

## Files, archives, and remote services

Local, archive, and remote entries share the same drag payload and transfer
confirmation. Writable destinations use the existing guarded transfer backends.
HTTP locations and unsupported archive formats reject writes; read-only sources
can be copied. A Move removes only completely transferred source items. Skipped
items stay at the source. Source changes or transfer failures prevent unsafe
source deletion; already completed copies may remain.

Archive transfers may require extraction and repacking. These phases use an
indeterminate progress indicator; ordinary local and remote transfers retain
byte-based progress when an accurate total is available. Eligible local operations
use the existing Undo journal. Transfers involving remote backends do not promise
a transactional Undo. Replaced archive contents have a durable whole-container
backup when a complete Undo is unavailable.

Archive mutation has the same limits as other archive commands: supported local
ZIP, TAR, and 7z containers can be edited; nested/remote containers and additional
read-only formats retain their existing limitations.

## Other applications and windows

Drag local files out of the window to start a native file drag. macOS/Windows
use AppKit/OLE; Linux uses the current X11 or Wayland surface rather than a helper
window. Ordinary dragging inside the window remains in Slint, including fast
press/move/release gestures. Files received from Finder, Explorer, or Linux file
managers use the same destination targets and confirmation.

Starting, hovering, leaving the window, or cancelling a drag never downloads or
extracts virtual file contents. Drops within Rust Commander open confirmation;
file contents are transferred only after you confirm. Conflict previews inspect
names and metadata, including nested merge conflicts, without exporting files.

Other applications consume real local file URLs. To export remote/archive entries
for those applications, explicitly choose **Selection → Prepare files for external
drag…** first. This named command downloads/extracts the selection in the background
and provides progress, **Cancel**, and **Escape**. Once “Files ready to drag” appears,
drag the prepared selection out of the window. An unprepared external virtual drag
shows that instruction and leaves the source untouched. There is no automatic
prefetch on a gesture; cross-platform lazy OS file promises are not implemented.

Native incoming drops acknowledge Copy while the asynchronous confirmation is
open. A requested Move is performed by Rust Commander's guarded transfer engine
after confirmation; cancelling cannot cause the other application to delete its
source. Native completion callbacks alone never delete originals.

For outgoing local drags, macOS/Windows advertise copy/move capabilities; the
receiving application determines its supported action. Linux outgoing native
file lists and virtual exports advertise Copy. Use Rust Commander's Move
confirmation for moves between its panes/windows. Native interoperation still
needs acceptance testing with the intended desktop/file manager; protocol tests
are not a substitute for Explorer, Nautilus, Dolphin, or Finder testing.

Prepared virtual exports live in the private `drag-exports` folder beside
`settings.json`. File URLs remain usable when a window closes or the app exits.
The next successful preparation removes this application's timestamped exports
older than 24 hours. You can remove that folder when no application is consuming
an exported drag. Cancelling an unfinished preparation removes its temporary
files and keeps the source.

## Implementation and validation

- `src/drag_drop.rs` implements immutable source selections, export, guarded
  cross-backend transfers, archive moves, and conflict discovery.
- `src/app/slint_ui/drag.rs` owns payload lifetimes, target routing, native hover,
  asynchronous exports, cancellation, and native handoff.
- `ui/drag-drop.slint` provides common targets, highlight, and hover timers;
  file-list edge targets handle scrolling.
- `src/native_drag/` provides the X11 adapter; `vendor/wayland-dnd/` records
  provenance, MIT/Apache licenses, bounded URI pipes, and the Wayland adapter.
  `vendor/drag/` contains the macOS/Windows source adapters.

Run ordinary regression checks with `cargo test --locked --all-targets
--all-features`, `cargo clippy --locked --all-targets --all-features -- -D warnings`,
and `cargo fmt --all -- --check`. On a Linux development machine with Xvfb,
XTEST and Weston installed, run:

```sh
./scripts/test-native-dnd.sh
```

That command runs the actual XDND source against a disposable target (Unicode,
large INCR payloads, cancellation), URI-list/pipe tests, and a headless Wayland
connection/lifetime test. CI runs these separately from ordinary application
checks. Consult [validation evidence](../parity/VALIDATION.md) for desktop checks
actually performed and remaining acceptance gaps.
