## Current review — 2026-10-21

This overview reflects the current source and the recorded checks in
[VALIDATION.md](VALIDATION.md). It separates absent behavior from behavior that
exists but still needs compatibility or native acceptance testing. This source
review did not perform new desktop or live-account tests. Historical validation
sections describe their original snapshots; later sections supersede older limits.

### Main implementation gaps

| Area | Existing behavior | Missing or limited behavior |
| --- | --- | --- |
| Drag and drop | Local-file copy payloads, pane drop confirmations, incoming OS handlers and explicit native drag-out | Move gestures; remote/archive sources; folder-row, tree and tab drop targets; hover navigation; seamless native drag-out. See the detailed matrix below. |
| Network services | HTTP/HTTPS, FTP/both FTPS modes, SFTP on all three desktop platforms, SMB2/3 and cloud adapters | NFS, Android/ADB, Windows Registry, oVirt/vSphere and Bonjour discovery. WebHDFS does not establish full Hadoop parity. |
| Archives | Create/extract common formats; edit local ZIP/TAR/7z; encrypted ZIP/7z; nested/remote browsing | Nested/remote editing, LST and libguestfs; archive links and special files. NRG/CPIO/RPM lack dedicated format fixtures. See [format support](../docs/FILE_OPERATIONS.md#archives). |
| Search and comparison | Local/remote search and separate local/remote synchronization | Local content search skips files over 16 MiB and matches literal text; Compare folders is local-only and skips symlinks. Full upstream search options remain open. |
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
| Local selection between panes | Slint file payloads and pane-wide copy drop targets; confirmation before transfer | Move negotiation/modifier semantics; real gesture-to-transfer completion across supported desktops |
| Finder/Explorer/Linux file manager into a pane | Incoming file events are grouped; destination pane is detected or explicitly confirmed through the active-pane fallback | End-to-end OS interoperability and correct targeting on native desktops, especially Wayland |
| Local files into a remote pane or open local archive | Upload confirmation or archive-add form; routing has callback coverage | Physical-drop completion with real destinations; folder/tab/tree targeting |
| Local files out to another application | Alt + drag or Selection → Native drag-out on macOS/Windows; separate GTK4 helper on Linux | Ordinary seamless drag-out, native source/destination completion and platform copy/move negotiation |
| Remote/archive entries as drag sources | No payload is provided for these sources | Downloads/extraction or virtual-entry payloads, plus source lifetime and cleanup |
| Folder rows, directory tree and tabs | Drops currently target the pane's open location | Dedicated targets, hover-to-open/tab switching and drag-specific scrolling |

Implementation evidence: [Slint drag/drop widgets](../ui/commander.slint),
[payload/native-event routing](../src/app/slint_ui.rs),
[transfer confirmation](../src/app.rs), [native adapters](../src/native_files.rs)
and [Linux helper](../src/linux_drag.rs).
Callback tests in [slint_ui/tests.rs](../src/app/slint_ui/tests.rs) cover confirmation,
modal guards and local-file drops into remote/archive destinations; pane-layout
hit testing is covered in [parity_tests.rs](../src/app/slint_ui/parity_tests.rs).
The GTK fixture checks the file-list payload and helper window. These checks do
**not** establish physical/native cross-application drop completion. Remote/archive
destinations accepting local files do **not** mean remote/archive drag sources work.

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
