## Current review — 2026-10-27

This overview reflects the current source and the recorded checks in
[VALIDATION.md](VALIDATION.md). It separates absent behavior from behavior that
exists but still needs compatibility or native acceptance testing. Drag/drop
includes the new checks recorded there; the other areas below remain source
reviews without new live-account acceptance. Historical validation
sections describe their original snapshots; later sections supersede older limits.

The reviewed source includes commit `6f78123`. The pinned inventory contains
**200 entries: 4 implemented, 188 partial, 7 missing and 1 inapplicable**.
These are 166 upstream actions and 34 modules, not 200 equally sized features.
In particular, **partial does not mean absent**: many entries have working
behavior and fixture coverage but lack complete upstream/native acceptance.
The inapplicable entry is the JVM garbage-collection command.

The latest full local run records **404 passing tests and 17 ignored**; macOS
and Linux-container Clippy pass. The connection-ID and Linux test-build fixes
are committed, but hosted Windows confirmation remains open. These checks do
not close the native/live-provider acceptance gaps below.

### Recently completed workflows

The following are implemented and should no longer appear as missing features:

- Flip/swap panes; Space/Insert marks with red highlighting, automatic cursor
  advance and marks preserved when clicking another row.
- Pack dialog with ZIP/7z/TAR/stream formats, compression levels, ZIP comments,
  foreground/background work, cancellation and expandable selected-file lists.
  The earlier generated-7z compatibility defect has a writer fix and independent
  extraction/integrity evidence; broader format compatibility remains open.
- Recovery manager with multi-selection restore, filtering, legacy backup import
  and reviewed cleanup; guarded nested/remote archive save-back.
- Copy names without extensions, a saved parent-row toggle, numbered window
  shortcuts, manual update checking, diagnostics and reviewable bug-report links.
- All eight formerly missing Commands entries now have implementations: bring
  all windows forward, Email files over encrypted SMTP, a bookmark explorer,
  documentation/community/website/donation links and an application event console.
  See [behavior and limits](../docs/DESKTOP_COMMANDS.md). Their partial statuses
  retain the remaining native/live-service acceptance work.

### Completely missing upstream entries

This table lists all 7 entries currently marked `missing` in `status.json`.
It excludes limitations in working features and validation-only gaps.

| Entry | Remaining gap |
| --- | --- |
| mucommander-protocol-nfs | NFS adapter. |
| mucommander-protocol-adb | Android/ADB device browsing. |
| mucommander-protocol-registry | Windows Registry virtual filesystem. |
| mucommander-protocol-ovirt | oVirt integration. |
| mucommander-protocol-vsphere | vSphere integration. |
| mucommander-format-lst | LST virtual archive/list format. |
| mucommander-format-libguestfs | Guest/disk-filesystem integration through libguestfs. |

### Main implementation gaps

| Area | Existing behavior | Missing or limited behavior |
| --- | --- | --- |
| Drag and drop | Local/remote/archive selections; guarded copy/move; folder/tab/tree/breadcrumb/sidebar targets; hover navigation and edge scrolling; native X11/Wayland/AppKit/OLE bridges | Native desktop acceptance remains incomplete; virtual exports are Copy. Deferred native providers exist, with preparation available for receivers that require pre-existing local paths. See the detailed matrix below. |
| Network services | HTTP/HTTPS, FTP/both FTPS modes, SFTP on all three desktop platforms, SMB2/3 and cloud adapters | The five missing adapters above and Bonjour discovery. WebHDFS does not establish full Hadoop parity. The Rust SSH transport does not interpret OpenSSH aliases or host certificates. |
| Archives | Pack dialog with ZIP/7z/TAR/stream formats, compression levels, ZIP comments, foreground/background work and expandable source list; guarded local/nested/remote editing; encrypted ZIP/7z | Packing takes local sources and a local output; direct remote/archive-source packing is absent. HTTP/format-override editing, LST/libguestfs and archive links/special files are unsupported. RAR/ISO/NRG/AR/DEB/CPIO/RPM creation/mutation is unavailable. Edited cached containers are limited to 128 MiB each, eight levels and a 512 MiB cache; cached edit recipes require reopening after restart. NRG/CPIO/RPM lack dedicated fixtures. See [format support](../docs/FILE_OPERATIONS.md#archives). |
| Search and comparison | Streaming local/remote literal and per-line regex search with size/date/exclusion filters; SHA-256 or explicitly unverified metadata comparison; bounded read-only text diffs; shared presets and filtered local/remote synchronization | Regex lines are bounded to 8 MiB; text differences to 1 MiB per side and bounded line alignment. Comparison excludes archive roots and reports links without following them. Synchronization refuses links/special files; filtered sync refuses directory deletion/type replacement. Broader provider/desktop acceptance remains open. |
| Viewers and editing | Encoded text editing, large local read-only pages, streaming local text/hex search, image/PDF rendering, PDF Unicode selection/copy and cancellable document search; reviewed external save-back to local/remote files and writable archive members | Built-in editing is limited to 2 MiB and does not edit archive members or symlinks; large local files are read-only. External working copies accept 32 MiB, but direct remote save-back still accepts only 2 MiB. Image/PDF input is limited to 32 MiB; large remote/archive paging, more encodings and image animation remain absent. OCR and PDF forms/annotation editing are also absent viewer enhancements, not additional missing inventory entries. |
| Recovery and publication | Journaled local Undo/Redo, retained remote originals, checkpointed copies and a restore/cleanup manager | Universal/remote Undo/Redo, crash recovery for temporary editor drafts and uninterrupted save-back sessions after restart are absent. Checkpointed copies refuse links; interrupted remote publication and concurrent writers remain provider-dependent, with no atomic guarantee on backends lacking conditional writes. Recovery restore selects at most 100 originals per operation. |
| Metadata | Native local moves; copying macOS ACLs/xattrs/resource forks, Linux user attributes/POSIX ACLs and Windows DACLs | Native ownership names/editing and full ACL presentation; Windows alternate data streams/ownership and privileged Linux namespaces are outside current copy preservation. Remote metadata depends on the provider. |
| Pane swapping and file marks | Opposing-arrow toolbar button and Navigate > Swap panes move tabs, cursor, marks and keyboard focus together; Space/Insert toggle a mark and advance in visible order, with red marks independent of the cursor highlight | Broader native desktop acceptance remains open. |
| Commands | Open As for explicit text/hex/image/PDF/archive interpretation, Open With, desktop reveal, searchable windows, configurable shortcuts, copy base names, parent-row toggle, numbered windows, manual release checks, diagnostics, reviewed bug-report links, bookmark explorer, bring-all-forward, Email files, support/donation links and event console | General selected-URL opening, dedicated shared Minimize/Maximize shortcut actions and complete upstream preference/shortcut coverage. Linux Reveal opens the parent folder rather than selecting the item. Native acceptance remains open. |
| Localization and migration | English/German/Spanish and reviewed XML import with backup | Additional languages, complete translated coverage and full upstream preference/keymap/command/bookmark import. See [import limits](../docs/FEATURES_0.7.md#12-mucommander-import). |

The size, link and publication guards above are deliberate current limits.
Removing them requires an appropriate streaming, recovery or filesystem design;
they are not claims that the existing guarded workflows are broken.

Upstream [Open As](https://github.com/mucommander/mucommander/blob/3504a51e9af9f69a67dc616db821a320ec780f79/mucommander-core/src/main/java/com/mucommander/ui/action/impl/OpenAsAction.java)
forces a format/extension interpretation without renaming the file. Rust Commander's
Open As now provides explicit viewer choices and read-only cached archive overrides;
see [limits](../docs/VIEWERS.md). Open With chooses an external application.

### Drag-and-drop parity: partial

Drag and drop is a cross-cutting workflow, not a separate named action in the
pinned upstream action inventory. It must remain visible here even though the
inventory counts do not include an additional DragDrop entry.

| Workflow | Current implementation | Missing behavior / acceptance still required |
| --- | --- | --- |
| Local selection between panes | Immutable multi-file payloads, copy/move confirmation and normal conflict/Undo guards | Broader physical gesture acceptance on Windows/Linux |
| Finder/Explorer/Linux file manager into a pane | Grouped file lists, frozen coordinates, highlighted folder/tab/tree/breadcrumb/sidebar targets; guarded confirmation | Real Explorer/Nautilus/Dolphin interoperation and compositor coverage |
| Transfers involving remote panes or local archives | Common transfer engine, per-item conflicts, completed-source-only deletion; atomic same-archive moves | Backend-specific live service acceptance; nested/remote archive mutation uses guarded outer-source save-back; HTTP/format overrides stay read-only |
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
  copies now retain fixed merge/Skip/Keep both/Replace decisions; recovery manager
  restoration and verified completed-batch cleanup are implemented.
- **Archives and viewers:** broader format, metadata, corruption/encryption and
  PDF compatibility fixtures remain necessary. Input/size bounds are deliberate
  current limits, not evidence of complete upstream parity.

[Remote connections](../docs/REMOTE_CONNECTIONS.md),
[current feature limits](../docs/FEATURES_NEXT.md) and
[stabilization coverage](../docs/STABILIZATION.md) provide the detailed behavior.
A missing Java module name can represent an internal architecture contract rather
than a missing user-facing protocol; the detailed entries below state that distinction.
