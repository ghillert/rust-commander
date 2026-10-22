## Current review — 2026-10-22

This overview reflects the current source and the recorded checks in
[VALIDATION.md](VALIDATION.md). It separates absent behavior from behavior that
exists but still needs compatibility or native acceptance testing. Drag/drop
includes the new checks recorded there; the other areas below remain source
reviews without new live-account acceptance. Historical validation
sections describe their original snapshots; later sections supersede older limits.

### Main implementation gaps

| Area | Existing behavior | Missing or limited behavior |
| --- | --- | --- |
| Drag and drop | Local/remote/archive selections; guarded copy/move; folder/tab/tree/breadcrumb/sidebar targets; hover navigation and edge scrolling; native X11/Wayland/AppKit/OLE bridges | Native desktop acceptance remains incomplete; virtual exports are Copy. Deferred native providers exist, with preparation available for receivers that require pre-existing local paths. See the detailed matrix below. |
| Network services | HTTP/HTTPS, FTP/both FTPS modes, SFTP on all three desktop platforms, SMB2/3 and cloud adapters | NFS, Android/ADB, Windows Registry, oVirt/vSphere and Bonjour discovery. WebHDFS does not establish full Hadoop parity. |
| Archives | Create/extract common formats; edit local ZIP/TAR/7z; encrypted ZIP/7z; nested/remote browsing | Nested/remote editing, LST and libguestfs; archive links and special files. NRG/CPIO/RPM lack dedicated format fixtures. See [format support](../docs/FILE_OPERATIONS.md#archives). |
| Search and comparison | Local/remote search, recursive SHA-256 comparison across local/remote panes, and reviewed local/remote synchronization with read-only destination guards | Local content search skips files over 16 MiB and matches literal text; comparison reports links without following them and excludes archive roots. Broader upstream search/comparison options and live-provider acceptance remain open. |
| Viewers and editing | Encoded text editing, large local read-only pages, streaming local text/hex search, image/PDF rendering | PDF text selection/search, additional encodings and animated images; large-file editing and large remote/archive viewing. Editing is bounded to 2 MiB, image/PDF input to 32 MiB. |
| Metadata | Native local moves; copying macOS ACLs/xattrs/resource forks, Linux user attributes/POSIX ACLs and Windows DACLs | Native ownership names/editing and full ACL presentation; Windows alternate data streams/ownership and privileged Linux namespaces are outside current copy preservation. Remote metadata depends on the provider. |
| Commands | Open With, desktop reveal, searchable windows, configurable shortcuts and native window controls | Forced-format Open As, copy base names, email, numbered window shortcuts, parent-entry visibility toggle, update checking and remaining help/diagnostic actions. |
| Localization and migration | English/German/Spanish and reviewed XML import with backup | Additional languages, complete translated coverage and full upstream preference/keymap/command/bookmark import. See [import limits](../docs/FEATURES_0.7.md#12-mucommander-import). |

Upstream [Open As](https://github.com/mucommander/mucommander/blob/3504a51e9af9f69a67dc616db821a320ec780f79/mucommander-core/src/main/java/com/mucommander/ui/action/impl/OpenAsAction.java)
forces a format/extension interpretation without renaming the file. It is distinct
from Rust Commander's existing Open With application chooser.

### Drag-and-drop parity: partial

Drag and drop is a cross-cutting workflow, not a separate named action in the
pinned upstream action inventory. It must remain visible here even though the
inventory counts do not include an additional DragDrop entry.

| Workflow | Current implementation | Missing behavior / acceptance still required |
| --- | --- | --- |
| Local selection between panes | Immutable multi-file payloads, copy/move confirmation and normal conflict/Undo guards | Broader physical gesture acceptance on Windows/Linux |
| Finder/Explorer/Linux file manager into a pane | Grouped file lists, frozen coordinates, highlighted folder/tab/tree/breadcrumb/sidebar targets; guarded confirmation | Real Explorer/Nautilus/Dolphin interoperation and compositor coverage |
| Transfers involving remote panes or local archives | Common transfer engine, per-item conflicts, completed-source-only deletion; atomic same-archive moves | Backend-specific live service acceptance; nested/remote archive mutation remains limited |
| Local files out to another application | Ordinary gesture handoff on leaving the window; native AppKit/OLE/XDND/Wayland sources without the Linux helper | Native Copy/Move negotiation implemented on all platforms; local delivery into Finder verified, with broader desktop acceptance still needed |
| Remote/archive entries as drag sources | In-window typed payloads; deferred AppKit/OLE/URI delivery after an accepted drop; shared cancellable exports and retained private files; gestures never fetch contents | Virtual exports are Copy. Private same-process window handoffs avoid preparation and exporting; an archive handoff was verified on Mac. Separate processes/receivers without deferred-format support need preparation; broader desktop acceptance remains open |
| Folder rows, directory tree, tabs, breadcrumbs and sidebar | Dedicated targets, 650 ms hover navigation/expansion, and file-list edge scrolling | Wider multi-scale/desktop acceptance |

Implementation: [gesture/widgets](../ui/drag-drop.slint),
[payload ownership and target routing](../src/app/slint_ui/drag.rs),
[guarded transfers](../src/drag_drop.rs), [native Linux bridge](../src/native_drag.rs),
and [usage/limits](../docs/DRAG_AND_DROP.md).
Pointer-event and callback tests cover initiation, selection lifetime, routing,
confirmation, cancellation and native hit feedback. Domain tests cover archive/remote
pairs, skipped moves, changed sources, recovery, progress and local Undo.
The standalone Linux fixtures exercise actual XDND small/INCR transfers under Xvfb
and Wayland connection lifetime under headless Weston. Native AppKit/OLE provider fixtures cover deferred writes and content gating. These checks do **not**
establish physical Explorer/Nautilus/Dolphin interoperability or every compositor.

### Validation gaps for existing implementations

- **Cloud accounts:** Dropbox/Google Drive/OneDrive OAuth, live S3/GCS and Hadoop
  still need real-account/service acceptance. Loopback token tests and a generic
  filesystem adapter are not provider certification.
- **Native desktop behavior:** Windows/Linux clipboard and drag interoperability,
  credential-store prompts, multi-monitor window behavior, full terminal
  compatibility and VoiceOver/NVDA/Orca still need broader interactive testing.
  CI builds/tests, Linux Xvfb checks and Mac smoke tests cover parts of this work.
- **Servers and failure modes:** recorded disposable SFTP/FTP/FTPS/WebDAV/SMB
  workflows do not cover every server, Windows OpenSSH-agent configuration,
  cross-volume failure or provider concurrency/publish behavior. Resumable remote
  copies currently require new targets without merge/replace choices.
- **Archives and viewers:** broader format, metadata, corruption/encryption and
  PDF compatibility fixtures remain necessary. Input/size bounds are deliberate
  current limits, not evidence of complete upstream parity.

[Remote connections](../docs/REMOTE_CONNECTIONS.md),
[current feature limits](../docs/FEATURES_NEXT.md) and
[stabilization coverage](../docs/STABILIZATION.md) provide the detailed behavior.
A missing Java module name can represent an internal architecture contract rather
than a missing user-facing protocol; the detailed entries below state that distinction.
