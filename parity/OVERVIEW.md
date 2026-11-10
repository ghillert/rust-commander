## Current review — 2026-10-31

This overview reflects the current source and the recorded checks in
[VALIDATION.md](VALIDATION.md). It separates absent behavior from behavior that
exists but still needs compatibility or native acceptance testing. Drag/drop
includes the new checks recorded there; the other areas below remain source
reviews without new live-account acceptance. Historical validation
sections describe their original snapshots; later sections supersede older limits.

The reviewed source includes commits `737eca4` and `3a684b6` and the current
Settings organization and native-control audit. The pinned inventory contains
**200 entries: 4 implemented, 190 partial, 5 missing and 1 inapplicable**.
These are 166 upstream actions and 34 modules, not 200 equally sized features.
In particular, **partial does not mean absent**: many entries have working
behavior and fixture coverage but lack complete upstream/native acceptance.
The inapplicable entry is the JVM garbage-collection command.

The latest recorded all-target runs have **460 passing tests and 18 ignored on
macOS**, and **455 passing tests and 23 ignored on Linux**. Native form fixtures,
Windows SDK/Wine checks and their limits are recorded in [VALIDATION.md](VALIDATION.md).
Hosted Windows confirmation and native/live-provider acceptance remain open.

### Recently completed workflows

The following are implemented and should no longer appear as missing features:

- Flip/swap panes; Space/Insert marks with red highlighting, automatic cursor
  advance and marks preserved when clicking another row.
- Pack dialog with ZIP/7z/TAR/stream formats, compression levels, ZIP comments,
  foreground/background work, cancellation and expandable selected-file lists.
  Local, remote and archive selections can be packed with ZIP/7z encryption to
  local or configured remote output; virtual sources are staged privately and
  verified before packing. Remote publication verifies the uploaded SHA-256.
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
- A general reviewed browser URL dialog, shared Minimize/Maximize shortcut
  actions and reviewed external-editor save-back through 2 GiB.
- Read-only NFSv3 export browsing and authorized Android browsing through a local
  ADB server. The Connections form can discover SMB, SFTP, FTP, and NFS
  advertisements via Bonjour and fill in the selected endpoint. See
  [network and device browsing](../docs/REMOTE_CONNECTIONS.md#network-and-device-browsing).
- Active sort indicators, a selectable `..` row, keyboard access to that row,
  focused-file actions independent of marks, dedicated file/background/parent,
  column and tab context menus, and a Duplicate tab command. See the interaction
  comparison below and the latest targeted checks in [VALIDATION.md](VALIDATION.md).
- Settings organized into General, Folders, Appearance, Shortcuts, Mail and
  Advanced tabs, with focused column/toolbar/command-bar subtabs, searchable
  primary/alternate bindings and shared Apply/OK/Cancel draft behavior. See
  [Settings organization](../docs/SLINT_UI.md#desktop-chrome). Settings and suitable
  operation/utility forms now use AppKit, Windows common controls or GTK4, with
  native local path choosers and a safe shared fallback. See the
  [native-control audit](../docs/NATIVE_FORMS.md).

### Current four-area implementation

SSH fingerprint review and alias/configuration resolution, ownership/ACL forms,
16 MiB built-in editing, 2 GiB external editing, 512 MiB image/PDF input,
remote/archive paging and built-in editor draft recovery are implemented. Windows
uses a Win32 main menu; shared/Linux menus support Alt/F10 navigation. System
high contrast and a shared-view override are connected. Current validation and
remaining real-desktop limitations are recorded in [VALIDATION.md](VALIDATION.md).

### Pane, keyboard and context-menu comparison — 2026-10-30

This behavioral review uses the pinned upstream source, rather than inferring
completion from action names. Reference points are
[FileTable](https://github.com/mucommander/mucommander/blob/3504a51e9af9f69a67dc616db821a320ec780f79/mucommander-core/src/main/java/com/mucommander/ui/main/table/FileTable.java),
[FileTableHeader](https://github.com/mucommander/mucommander/blob/3504a51e9af9f69a67dc616db821a320ec780f79/mucommander-core/src/main/java/com/mucommander/ui/main/table/FileTableHeader.java),
[TablePopupMenu](https://github.com/mucommander/mucommander/blob/3504a51e9af9f69a67dc616db821a320ec780f79/mucommander-core/src/main/java/com/mucommander/ui/main/menu/TablePopupMenu.java),
[ShowFilePopupMenuAction](https://github.com/mucommander/mucommander/blob/3504a51e9af9f69a67dc616db821a320ec780f79/mucommander-core/src/main/java/com/mucommander/ui/action/impl/ShowFilePopupMenuAction.java)
and the upstream tab popup and cloned-tab factory.

| Interaction | Review result |
| --- | --- |
| Cursor versus marks | Fixed Open, Open As/With, View/Edit, external edit, Rename and folder-opening actions to use the focused row. Copy/Move/Trash/Pack retain the persistent marked selection. |
| Parent row | Up/Home and page/block/range movement can reach `..` when enabled. Space advances without marking it; Enter opens it. The parent never becomes a file-operation source. |
| Returning to a parent | The child folder/archive is selected after going up, including cached archive source identities. |
| Re-entering the same location | Refresh preserves cursor, marks and filter rather than clearing them. |
| Selection gestures | Ordinary clicks retain marks; modifier-click/range selection and middle-click marking remain independent of opening. Held Space no longer repeatedly toggles the final row. |
| Sorting | Header clicks focus and activate the clicked pane. Sort direction remains specific to each pane/tab. Resizing uses its separate handle. |
| File context | Added focused-file Open and clipboard/name/path/marking actions, while retaining transfer, packing and metadata workflows. Global connection/settings/import commands remain in the main menus. |
| Parent/background contexts | Right-clicking `..` highlights it without navigating. Empty-space menus activate the clicked pane and clear stale cursor targets; explicit file marks remain usable. |
| Column context | Right-click a header to toggle columns and automatic sizing, with checked states. Name stays mandatory. |
| Tab context | Right-click targets the clicked tab and exposes duplicate, clone, name, lock, move and close actions. Duplicating/cloning retains the custom title and lock. Middle-click closes a tab while retaining at least one per pane. |
| Tab creation/focus | New, duplicate, clone and open-in-new-tab honor the existing 32-tab limit. Moving a tab to the other pane also moves keyboard focus. |
| Keyboard context menus | Shift+F10 anchors to the cursor row. Opening is deferred outside controller borrows. Arrows/Home/End, item-name jumps, scrolling, Enter/Space and Escape are handled inside the popup; disabled entries are skipped. |
| Read-only locations | Mutating commands are disabled for HTTP locations; local-only file creation/metadata/permanent-delete commands are disabled for remote locations. |
| Text fields and customization | Existing address/filter editing, their context menus, configured shortcuts, quick find and Space/Insert marking retain their own focus rules. Pane switching respects its configured Tab binding. |

Regression coverage is in `src/app/slint_ui/interaction_tests.rs`, the existing
context-menu/parity tests and `tests/features_07.rs`. Rename and metadata edits
continue through the existing form dialogs; this is a functional interaction
comparison, not a claim of identical Swing widget presentation. Native
Windows/Linux pointer, input-method and assistive-technology acceptance remains
open. The broader feature limits below still apply.

### Completely missing upstream entries

This table lists all 5 entries currently marked `missing` in `status.json`.
It excludes limitations in working features and validation-only gaps.

| Entry | Remaining gap |
| --- | --- |
| mucommander-protocol-registry | Windows Registry virtual filesystem. |
| mucommander-protocol-ovirt | oVirt integration. |
| mucommander-protocol-vsphere | vSphere integration. |
| mucommander-format-lst | LST virtual archive/list format. |
| mucommander-format-libguestfs | Guest/disk-filesystem integration through libguestfs. |

### Main implementation gaps

| Area | Existing behavior | Missing or limited behavior |
| --- | --- | --- |
| Drag and drop | Local/remote/archive selections; guarded copy/move; folder/tab/tree/breadcrumb/sidebar targets; hover navigation and edge scrolling; native X11/Wayland/AppKit/OLE bridges | Native desktop acceptance remains incomplete; virtual exports are Copy. Deferred native providers exist, with preparation available for receivers that require pre-existing local paths. See the detailed matrix below. |
| Network services | HTTP/HTTPS, FTP/both FTPS modes, SFTP, SMB2/3, cloud adapters, read-only NFSv3 and Android browsing, and Bonjour discovery for SMB/SFTP/FTP/NFS | Windows Registry, oVirt and vSphere remain missing. NFS requires a known export and an unprivileged TCP connection. Android requires a local ADB server and file-sync v2. Writes and broader server/device/discovery acceptance remain open. WebHDFS covers part of Hadoop support. SFTP supports OpenSSH aliases and host fingerprint review. SSH certificates, jump/proxy commands and keyboard-interactive/MFA remain unsupported. |
| Archives | Pack dialog with ZIP/7z/TAR/stream formats, compression levels, ZIP comments, foreground/background work and expandable source list; guarded local/nested/remote editing; encrypted ZIP/7z | Packing takes local, remote or archive sources, ZIP/7z encryption and local or configured remote output. Remote publication follows verified staging and provider-specific create guards; remote output has no general Undo. Live-provider acceptance remains incomplete. HTTP/format-override editing, LST/libguestfs and archive links/special files are unsupported. RAR/ISO/NRG/AR/DEB/CPIO/RPM creation/mutation is unavailable. Edited cached containers are limited to 128 MiB each, eight levels and a 512 MiB cache; cached edit recipes require reopening after restart. NRG/CPIO/RPM lack dedicated fixtures. See [format support](../docs/FILE_OPERATIONS.md#archives). |
| Search and comparison | Streaming local/remote literal and per-line regex search with size/date/exclusion filters; SHA-256 or explicitly unverified metadata comparison; bounded read-only text diffs; shared presets and filtered local/remote synchronization | Regex lines are bounded to 8 MiB; text differences to 1 MiB per side and bounded line alignment. Comparison excludes archive roots and reports links without following them. Synchronization refuses links/special files; filtered sync refuses directory deletion/type replacement. Broader provider/desktop acceptance remains open. |
| Viewers and editing | Encoded text editing, local/remote/archive read-only pages, streaming text/hex search, image/PDF rendering, PDF Unicode selection/copy and cancellable document search; reviewed external save-back to local/remote files and writable archive members | Built-in editing accepts 16 MiB and does not edit archive members or symlinks. Local/remote/archive read-only paging and streaming search/jump are implemented; archive snapshots are capped at 2 GiB. Reviewed external editing accepts 2 GiB; image/PDF file-backed input accepts 512 MiB with decoded limits. More encodings and image animation remain absent. OCR and PDF forms/annotation editing are also absent viewer enhancements, not additional missing inventory entries. |
| Recovery and publication | Journaled local Undo/Redo, retained remote originals, checkpointed copies and a restore/cleanup manager | Built-in editor drafts have private debounced autosave and guarded recovery. Universal/remote Undo/Redo, external-editor draft recovery and uninterrupted external save-back sessions after restart remain absent. Checkpointed copies refuse links; interrupted remote publication and concurrent writers remain provider-dependent, with no atomic guarantee on backends lacking conditional writes. Recovery restore selects at most 100 originals per operation. |
| Metadata | Native local moves; copying macOS ACLs/xattrs/resource forks, Linux user attributes/POSIX ACLs and Windows DACLs | Readable owner/group names, symbolic/octal permissions and local ACL presentation are available. A native local form now edits owner/group and macOS/POSIX/Windows ACL rules with stale-object guards. Remote editing, Windows alternate data streams/ownership preservation and privileged Linux namespaces remain outside current copy preservation. Remote metadata depends on the provider. |
| Pane swapping and file marks | Opposing-arrow toolbar button and Navigate > Swap panes move tabs, cursor, marks and keyboard focus together; Space/Insert toggle a mark and advance in visible order, with red marks independent of the cursor highlight | Broader native desktop acceptance remains open. |
| Commands | Open As for explicit text/hex/image/PDF/archive interpretation, Open With, desktop reveal, searchable windows, configurable shortcuts, copy base names, parent-row toggle, numbered windows, manual release checks, diagnostics, reviewed bug-report links, bookmark explorer, bring-all-forward, Email files, support/donation links, reviewed general HTTP/HTTPS browser opening, shared Minimize/Maximize actions and event console | Complete upstream preference/shortcut coverage and broader parameterized URL integration. Linux Reveal opens the parent folder rather than selecting the item. Native acceptance remains open. |
| Localization and migration | English/German/Spanish and reviewed XML import with backup | Additional languages, complete translated coverage and full upstream preference/keymap/command/bookmark import. See [import limits](../docs/FEATURES_0.7.md#12-mucommander-import). |
| Desktop appearance and interaction | Actual AppKit/Win32/GTK4 Settings and suitable short forms, with native path choosers and shared operation logic; custom file-manager chrome retains Cupertino/Fluent/Cosmic styling | Physical desktop screen-reader, mixed-DPI and high-contrast acceptance; complete Windows dark-mode support, compositor coverage and KDE widget matching remain open. Windows has a native menu bar; Linux/shared menus have Alt/F10 navigation. Windows/Linux high-contrast detection and a shared-view override are implemented. Unsupported native parenting uses Slint. See the [native-control audit](../docs/NATIVE_FORMS.md) and [desktop UX review](../docs/DESKTOP_UX_REVIEW.md). |

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
