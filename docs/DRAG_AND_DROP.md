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

Remote and archive entries can be dragged directly into applications that accept
virtual files. No manual preparation is required for these targets:

| Platform | Deferred delivery |
| --- | --- |
| macOS | AppKit file promises; the receiver supplies a destination after accepting the drop. Files/folders publish from private staging without overwriting a racing destination. |
| Windows | Shell `FileGroupDescriptorW` metadata and `FileContents` streams; content requests are rejected until the native source commits an accepted drop. Folder descriptors include their descendants. |
| Linux X11 | URI-list requests during hover wait; exports begin after an accepting target receives `XdndDrop`. Small and incremental URI lists are supported. |
| Linux Wayland | Early URI-list pipes are queued; exports begin only after the compositor's `dnd_drop_performed` event. The event queue stays responsive. |

Metadata enumeration can list remote directories, but does not fetch file bodies.
Concurrent content requests share one export. Virtual exports advertise Copy and
never delete remote/archive sources. Progress and Cancel/Escape cover downloading,
extraction and AppKit's staged delivery. Once file URLs/streams have been handed
to another application, its own progress/cancellation controls govern its copy.
Cancelling or failing an export removes private staging and leaves originals;
already delivered files may remain at the receiver. An idle receiver that stops
requesting promised entries times out rather than blocking the window indefinitely.

**Selection → Prepare files for external drag…** remains a fallback for applications
that require existing local paths or inspect file URLs before accepting a drop.
It also avoids waiting for a large export in a Linux receiver's URI-list timeout.
Prepare the selection, wait for “Files ready to drag”, then drag it out. Prepared
file URLs also support receivers in a separate Rust Commander process.
Windows virtual descriptors reject symlinks and paths exceeding the Shell's
descriptor-name limit; use an ordinary local copy when a selection cannot be
represented safely.

Windows advertises `IDataObjectAsyncCapability` so compatible Shell receivers
can extract on a background worker. For synchronous receivers, exports still
run off-thread while COM dispatches window messages and calls, allowing repaint,
progress, and cancellation. Native Windows desktop responsiveness still requires
acceptance testing; cross-compilation alone does not establish it.

Dragging a remote/archive selection between windows in the **same running
process** needs no preparation. A short-lived, single-use private token carries
the original selection to the destination window. Hovering and dropping do not
export or download contents; dropping opens the normal confirmation. Cancel
leaves both sides unchanged. AppKit/OLE use private native metadata, X11 sends a
private marker only to this process's known windows, and Wayland negotiates a
private MIME type only for an active in-process offer. Late content requests
cannot turn a private handoff into an external export. Separate processes and
applications that cannot accept deferred formats retain the preparation fallback.

Native incoming drops acknowledge Copy while the asynchronous confirmation is
open. A requested Move is performed by Rust Commander's guarded transfer engine
after confirmation; cancelling cannot cause the other application to delete its
source. Native completion callbacks alone never delete originals.

Outgoing local drags advertise Copy/Move on macOS, Windows, X11 and Wayland; the
receiving application negotiates and performs the supported operation. Rust
Commander never deletes originals just because a native completion callback says
Move. Native interoperation still needs acceptance testing with the intended
desktop/file manager; protocol tests do not establish Explorer, Nautilus, Dolphin
or Finder compatibility by themselves.

Prepared virtual exports live in the private `drag-exports` folder beside
`settings.json`. File URLs remain usable when a window closes or the app exits.
The next successful preparation removes this application's timestamped exports
older than 24 hours. Automatic deferred exports use the same retention rule. You can remove that folder when no application is consuming
an exported drag. Cancelling an unfinished preparation removes its temporary
files and keeps the source. Export leases protect active workers; expired,
interrupted exports with an application lease can be cleaned on the next export.

## Implementation and validation

- `src/drag_drop.rs` implements immutable source selections, export, guarded
  cross-backend transfers, archive moves, and conflict discovery.
- `src/drag_drop/promises.rs` owns deferred sessions, shared exports, cancellation
  and atomic AppKit publishing.
- `src/app/slint_ui/drag.rs` owns payload lifetimes, target routing, native hover,
  asynchronous exports, cancellation, and native handoff.
- `ui/drag-drop.slint` provides common targets, highlight, and hover timers;
  file-list edge targets handle scrolling.
- `src/native_drag/` provides the X11 adapter; `vendor/wayland-dnd/` records
  provenance, MIT/Apache licenses, bounded URI pipes, and the Wayland adapter.
  `vendor/drag/` contains the macOS/Windows source and drop-receiver adapters.
  The macOS receiver accepts modern file URLs while preserving the existing
  winit delegate's remaining behavior.

Run ordinary regression checks with `cargo test --locked --all-targets
--all-features`, `cargo clippy --locked --all-targets --all-features -- -D warnings`,
and `cargo fmt --all -- --check`. On a Linux development machine with Xvfb,
XTEST and Weston installed, run:

```sh
./scripts/test-native-dnd.sh
```

That command runs the actual XDND source against a disposable target (Unicode,
large INCR payloads, deferred hover probes, copy/move and cancellation), URI-list/pipe tests, and a headless Wayland
connection/lifetime test. CI runs these separately from ordinary application
checks. macOS and Windows CI also run `cargo test --locked -p
rust-commander-native-dnd-tests -- --test-threads=1` for the AppKit/OLE provider
fixtures. Consult [validation evidence](../parity/VALIDATION.md) for desktop checks
actually performed and remaining acceptance gaps.
