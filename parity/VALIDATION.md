# Validation record

## URL/window commands, virtual packing and external remote saves — 2026-10-30

The Apple Silicon macOS all-target/all-feature suite passes **411 tests with 17
ignored** across 31 suites. The additional selected-HTTP-entry URL assertion also
passes in the targeted dialog fixture. Clippy with warnings denied passes on
macOS and in the Linux container; formatting and diff whitespace checks pass.
The macOS development app builds and passes strict code-signature verification;
the existing debug-linker compact-unwind warning remains.

New coverage verifies:

- General HTTP/HTTPS URL review, scheme/credential refusal, selected/current HTTP
  prefill, menu/shortcut registration and Minimize/Maximize shortcut dispatch.
  Window actions remain available while a dialog is open.
- Remote filesystem-provider and archive-prefix packing to local ZIP/7z/TAR.GZ/
  GZip outputs, Unicode names, selected folder trees, compression/comment options,
  serialized queue recipes, cancellation, unsafe-path/conflict/stream-folder
  refusal, unchanged sources and destination-only local Undo. Headless callbacks
  exercise marked virtual selections with actual foreground/background workers.
- Direct remote external-editor save-back at the exact 32 MiB boundary, retained
  verified recovery bytes, changed-source/oversize/cancellation refusal and buffer
  exclusion from queue JSON. Built-in remote saves retain their 2 MiB bound.

The longer File popup exposed a fixture assumption that every entry was visible.
Context-menu tests now scroll with pointer events before clicking off-screen
commands; all six context-menu tests pass, including every action in both panes.

The URL form, archive-source Pack form and Window menu were rendered using
software/light and FemtoVG/dark. All six screenshots were visually reviewed with
disposable files/configuration; the screenshot workflow never opens the browser
or packs into user folders. Rendering allows 60 seconds for debug-app startup.

Remote fixtures use the filesystem provider, not live storage accounts. Native
browser/window-manager acceptance and broader provider concurrency remain open.
Packing output remains local, and encrypted creation still requires local sources.
These changes preserve the inventory counts: **4 implemented, 188 partial,
7 missing and 1 inapplicable** across 200 entries.

## Remaining desktop commands — 2026-10-27

All eight formerly missing Commands entries now have implementations: bring all
windows forward, Email files, Explore bookmarks, documentation/community/website
links, Donate and the debug console. The native and in-window Help menus now
retain the full command list. See [behavior and limits](../docs/DESKTOP_COMMANDS.md).

The Apple Silicon macOS all-target/all-feature run passes **404 tests with 17
ignored** across 29 suites. Clippy with warnings denied passes on macOS and in
the Linux container. Formatting and diff whitespace checks pass; the development
macOS app bundle builds and passes strict code-signature verification. The debug
linker reports the existing large `__eh_frame` compact-unwind warning.
New coverage includes eight domain/integration tests, six
headless dialog tests and a three-window dispatch test. Fixtures check MIME
attachment bytes and header validation, bounded local/remote/archive preparation,
cancellation, refusal to downgrade SMTP without STARTTLS, settings persistence
without passwords, bookmark edits and rollback/stale guards, support-link
validation, console filtering/privacy, pending-send controls and window order.
The transport test uses loopback SMTP and never sends an external email.

The six new screenshot cases in `scripts/management-smoke.py` render bookmarks,
email, SMTP settings, console, donation and the complete Help menu. All twelve
1240×780 software/light and FemtoVG/dark images were visually reviewed using
disposable configuration and files. They do not send email or open donation
pages. The donation default follows the published repository funding declaration
for GitHub Sponsors (`ghillert`); the URL remains configurable.

This establishes implementation and fixture coverage, not live SMTP delivery,
system-browser launch or native window-focus acceptance across platforms. These
eight entries therefore remain partial in the parity inventory. Counts are now
**4 implemented, 188 partial, 7 missing and 1 inapplicable** across 200 entries;
the seven missing entries are protocol/format integrations.

## Windows HTTP and Linux Clippy follow-up — 2026-10-26

[CI run 40407268454](https://github.com/ghillert/rust-commander/actions/runs/40407268454)
failed on Windows while downloading an HTTP index member and on Linux because
the physical drag-button polling method became unused in headless test builds.
That method now follows its caller's `not(test)` configuration.

Connection defaults previously used only wall-clock timestamps as IDs. Equal
timestamps could overwrite another connection's profile/session in the shared
maps. Defaults now use random 128-bit IDs; existing saved IDs remain unchanged.
A regression creates 16 simultaneous loopback HTTP connections, checks distinct
profiles and downloads, and verifies serialized IDs round-trip. A separate
Windows binary run under Wine produced 60,544 duplicate timestamp IDs from
65,536 attempts. This confirms the collision risk, but does not conclusively
identify the original Windows request error; hosted Windows verification remains
pending.

All-target/all-feature tests pass on Apple Silicon macOS: **389 passed, 17
ignored**. Clippy with warnings denied passes on macOS and in the Linux test
container; formatting and diff whitespace checks pass as well.

## Recovery manager, archive save-back and desktop commands — 2026-10-26

The Apple Silicon Mac run passes **388 tests with 17 ignored** across all
targets/features. Formatting, diff whitespace checks and Clippy with warnings
denied pass. New fixtures cover:

- Recovery discovery from journals, checkpoints and indexed backups; multi-file
  Keep-both restoration; cleanup groups and latest/pinned protection; changed or
  forged snapshots; legacy imports; incomplete/unknown checkpoints and link escapes.
- Nested ZIP/TAR/7z Add/Remove/Rename, encrypted nested containers, external-editor
  save-back, local outer-file Undo, changed-source and cancellation guards, and
  rejection of edits prepared before another save. Remote archive fixtures use a
  filesystem provider, exceed the text editor's 2 MiB limit and verify retained
  server originals. Replacement drops retain the actual outer archive rather
  than a temporary cached copy and do not promise Undo for remote publication.
- Base-name copying, parent-row preferences and all ten numbered-window shortcut
  dispatches; diagnostics privacy and report URL construction; asynchronous
  update results and errors. Loopback release checks cover valid responses,
  missing releases, refusal/rate limits, redirects, failures and oversized bodies.

`scripts/management-smoke.py` renders the parent row, selected recovery originals,
the nested archive source/save-back form, diagnostics and report dialogs using
software/light and FemtoVG/dark rendering. The ten 1240×780 images were visually
reviewed, including the two-row recovery footer. Screenshot callbacks exercise
real UI handlers with disposable data; they do not post an issue or install an
update. The parity audit now records 4 implemented, 180 partial, 15 missing and
1 inapplicable entry across 200 upstream entries.

These results do not establish Windows/Linux native behavior, physical numbered
window focus, browser launch/submission or remote-provider concurrency guarantees.
Provider acceptance beyond the filesystem fixture remains open; services without
conditional writes cannot atomically exclude external writers. The release
checker was tested against loopback responses rather than the live GitHub API.

## Path handling audit — 2026-10-26

The follow-up source audit reviewed native/portable relative-path boundaries,
archive member validation, remote keys and stamps, transfer/recovery paths,
comparison/synchronization inventories, URI parsing/serialization, startup
arguments, clipboard/drag paths and saved location identities. It found and fixed
these additional issues:

- Nested remote/archive drags and archive drop destinations, conflict overrides
  and move cleanup passed native paths into the raw archive-header validator.
  They now use native-to-portable validation. Raw archive headers still reject
  backslashes, drive prefixes, traversal and alternate-stream syntax.
- Fallback `remote://` addresses did not escape literal URI characters. They now
  round-trip Unicode, spaces, `#`, `?` and literal percent sequences. Legacy and
  protocol addresses share segment decoding, so encoded separators are rejected;
  legacy schemes also accept uppercase spelling and reject fragments.
- HTTP mount roots treated literal percent sequences as already escaped. Root
  components now use URL segment encoding while preserving encoded endpoint
  components. Parent traversal in a configured HTTP root is rejected explicitly.
- Startup arguments lost non-Unicode filename bytes through a display conversion.
  They now stay native paths. Component-based home expansion also prevents
  `~//folder` from discarding the home directory and accepts native Windows
  separators after `~`.

Regression fixtures cover nested remote exports, nested archive moves, per-file
Skip decisions, unsafe virtual selections, URI round-trips, a real loopback HTTP
request with an encoded mount name, and native startup/home paths. This is a
source audit and fixture validation; Windows execution still requires CI.

The final Apple Silicon Mac run passes 364 tests with 17 ignored across all
targets/features. Formatting and Clippy with warnings denied also pass.

During final validation, the released-gesture headless test observed the physical
desktop mouse button instead of only its simulated input. Headless UI tests now
use their explicit gesture state; application builds still poll native input.

## Remote recovery path normalization — 2026-10-26

Windows CI [run 40378213497](https://github.com/ghillert/rust-commander/actions/runs/40378213497/job/121767193652)
failed during the mixed-conflict checkpoint transfer test. Remote destination
stamps hashed native and portable path spellings differently, making an unchanged
destination appear modified. Stamps now hash the canonical remote object key;
metadata and content checks remain in place.

A regression test reproduced the mismatch locally before the fix. It covers
equivalent file/directory paths, Windows separators on Windows, and content edits
with unchanged size and timestamp. On the available Apple Silicon Mac, the full
all-target/all-feature suite passes 355 tests with 17 ignored; formatting and
Clippy with warnings denied also pass. This includes the existing transfer,
interrupted recovery, and synchronization tests. Windows execution of this
revision still requires a CI rerun. Existing recovery receipts containing a
noncanonical stamp continue to refuse a mismatch and retain their recovery files
for review.

## Archive interoperability and viewer tools — 2026-10-25

On the available Apple Silicon Mac, the current source passes 354 tests with
17 ignored, plus formatting and Clippy for all targets/features with warnings
denied. One ignored test was run separately with official 7-Zip 26.05 and passed.
The normal CI matrix still needs to validate this revision on Windows/Linux.

- Generated 7z archives extract with libarchive 3.8.5 through compress-tools;
  system bsdtar (libarchive 3.7.4) also extracts a generated file. Fixtures cover
  single files, nested folders, Unicode paths, zero-valued CRCs, empty entries and
  default/fastest/maximum levels. Ordinary entries are checked for incorrect
  anti-item flags. The pinned writer patch and its scope are recorded in
  [vendor notes](../vendor/sevenz-rust/PATCHES.md).
- Official 7-Zip integrity checks and full extraction preserve plain and
  AES-encrypted folder contents at all three tested levels, including empty
  directories. The optional external-tool test is
  `official_sevenzip_checks_plain_and_encrypted_output`; normal CI always runs
  the independent libarchive tests without needing a separate CLI installation.
- Open As domain/UI checks cover misleading extensions, text/image/PDF/hex
  overrides, bounded large-file handling, nested read-only archive copies,
  original parent navigation, ZIP/7z password validation and frozen selection.
  Right-click menu dispatch/cancellation coverage includes the new command.
- PDF tests cover forward/backward/wrapped searches, encrypted documents,
  cancellation, UTF-8 offsets for Unicode/ligatures/surrogates, rotated selection,
  and physical pointer selection at fit-width. Text view remains read-only.
- Ten isolated macOS previews (software/femtovg; Open As, archive password,
  highlighted search, text view and rotated dark view) rendered successfully.
  The development app bundle was rebuilt and ad hoc signed without restarting
  the user's running app.
- Windows CI run 40293595998 exposed a receipt-lookup test that compared a native
  serialized path with a Unix-only string. The lookup now compares paths;
  `transfer_controls` passes locally (8 passed, 1 ignored), with targeted Clippy
  and formatting clean. A Windows CI rerun remains necessary.

These are fixture and local desktop results. OCR, forms/annotation editing,
complex PDF reading order and broad archive/encryption variants remain outside
this validation. See [viewer limits](../docs/VIEWERS.md).

This record covers versions 0.3–0.6 on the available Apple Silicon Mac,
including Linux checks in an ARM64 Debian container. The latest interface
validation is in [Slint migration (0.6.0)](#slint-migration-060); earlier sections
retain historical results for the previous frontend. This is not a full
muCommander compatibility certification. See [PARITY.md](PARITY.md) for
implementation gaps, including missing behavior as well as unverified platforms.

## Automated checks

- The 0.3.0 baseline had 79 passing tests: 26 core file-manager tests, 18 desktop/archive/viewer tests,
  12 transfer-conflict tests, 3 extended-format tests, 2 worker-failure tests,
  1 real HTTP WebDAV test, 2 egui drag/drop tests, 5 appearance tests,
  5 settings migration tests, and 5 credential compatibility tests.
- Rust formatting and Clippy with warnings denied pass.
- Optimized macOS app bundle builds, and its ad hoc signature verifies.
- Version 0.4.0 passes 114 automated tests, plus a separate interactive Mac
  clipboard test. It adds recovery, conflicts, ZIP mutation/encryption, nested/remote
  archives, OAuth, localization and preference coverage; see the section below.
- The parity gate intentionally fails until the remaining upstream items have
  implementation and validation evidence. Its item counts are not a percentage
  of the product's functionality.

The WebDAV test binds an ephemeral loopback port. On this machine it requires
running outside the network sandbox. It does not contact a live account.
Tests use disposable temporary directories; the RAR/ISO/PDF fixtures and their
origins are documented in NOTICE.

## Native macOS smoke checks

The packaged app was operated through its native window. File-operation checks
began in 0.2.0; the appearance-specific checks below use 0.3.0:

| Workflow | Observed result |
| --- | --- |
| Launch | App opens and renders both panes with saved locations. |
| PDF viewing | Opened the two-page fixture and displayed its second page. |
| Embedded terminal | Executed a printf command and displayed its output. |
| Automatic refresh | A newly created test file appeared without manual refresh. |
| Recursive search | `*.pdf` returned only the PDF fixture. |
| Preferences | Font size, visibility, refresh and shortcut controls displayed; dismissed without applying changes. |
| Recoverable replacement | Copied a conflicting test file using Replace; result displayed the recovery path. Filesystem checks confirmed source preservation, new destination content and the old content in its backup. |
| Address shortcut | Cmd+L selected the existing address; pasting a new absolute path replaced it and navigated successfully. |

## Historical validation limits (0.4.0)

Windows builds and Windows/Linux native desktop interactions still need
validation. Linux Clippy and tests pass in an ARM64 Debian container on this Mac;
that does not validate an Ubuntu release build or native Linux desktop behavior.
CI remains configured for all three platforms. Live SFTP, FTP, S3, cloud OAuth, keychain UI and other remote
services have not been validated. The native automation drag gesture did not
produce a drop; pane and external drops pass egui event tests, while native
desktop drag/drop integration remains unverified. The filesystem adapter and loopback WebDAV
server establish only their tested contracts. Mac native file clipboard is tested. Drag-out still needs native end-to-end
validation, and Linux native drag-out is not implemented. Local Undo, ZIP editing,
encryption and initial localization are implemented; Redo and broader upstream
workflows remain open. Format-edge-case coverage is incomplete.

## Desktop appearance validation (0.3.0)

The Mac bundle displays its native AppKit menus, compact toolbar, installed
San Francisco font, blue folder icons, striped rows and tinted Favorites sidebar.
Cmd+, opens the new appearance settings. The profile popup displays Automatic,
macOS, Windows, Ubuntu and Linux/GNOME choices. Automated tests cover backward
compatible settings, theme changes from the system, explicit overrides, profile
persistence, available-font rendering, and selected-filename contrast.

Cross-platform palettes are implemented, but actual Windows and Linux appearance
and desktop integration have not been tested on those operating systems. The UI
uses egui-drawn controls with native window frames, plus native menus on macOS;
it does not replace every control with a platform widget.

The final Mac window extends the toolbar into the native title bar, leaving
space for AppKit's window controls. Accessibility inspection confirms close,
minimize and full-screen/zoom controls remain present. The packaged app was
previewed with the Ubuntu profile and with macOS dark appearance; settings
apply correctly, and the fixed Cancel/Apply footer stays visible while scrolling.
The window uses native OS controls with a custom toolbar; toolbar-background
drag handling is implemented but has not been verified with a physical mouse.

## Rust Commander naming

The application, Cargo package/executable, macOS bundle, CI artifacts and
current documentation now use Rust Commander / `rust-commander`. The optimized
bundle's display name, executable and `io.github.ghillert.rust-commander`
identifier are verified; its ad hoc signature verifies, and CLI help/version
output uses the new name. The native UI smoke checks above were performed
before this naming change.

Compatibility tests confirm that existing preview settings are read when the
new configuration file is absent, current settings take precedence, and invalid
settings remain preserved and reported. Credential tests use isolated in-memory
keyring entries: legacy fallback, current-entry precedence, lookup errors,
invalid data, and deletion of both namespaces are covered. They do not validate
live operating-system credential prompts or access controls.

## Twelve-feature release (0.4.0)

The [feature guide](../docs/FEATURES_0.4.md) maps the twelve requested areas to
implemented behavior and limits. Automated checks cover mixed per-file conflicts,
local Undo (moves, renames, replacements, merges and exact ZIP restoration),
external-edit and backup-tamper refusal, consecutive directory Undo, cancelled
operations and orphan-record recovery, ZIP editing, AES ZIP/7z round trips and
wrong passwords, nested archive Parent navigation, remote files/folders and
archive browsing with the filesystem adapter, PKCE/state validation, loopback
token exchange/redaction/fragmented HTTP, preferences and localization.

The real Mac clipboard test preserves its prior contents in memory and exercises
two files including a Unicode filename: Copy, Cut, selection ownership, external
Copy clearing Cut semantics, and external text clearing a remote selection. It
passes after fixing AppKit UTI identifiers and Unicode filename equivalence.
The test is ignored in normal CI because it requires an interactive desktop.

Live OAuth accounts and Windows/Linux desktop interactions remain untested.
The build uses X11 and Wayland clipboard backends, subject to compositor support;
Linux native drag-out falls back to copying files to the clipboard.

### Native 0.4.0 smoke checks

A separate test bundle used disposable files and isolated configuration, leaving
the user's running application and settings intact. Native checks confirmed:

- Copy confirmation and completed queue/history; Cmd+Z removed the copied file.
- F2 inside a ZIP opened Archive tools; renaming an entry changed the archive and
  listing, and Undo restored the original entry. Filesystem checks confirmed both.
- German applied to toolbars, sidebar, native menus, preferences and help.
- Settings still opened after a language change. This caught a native menu receiver
  bug; labels now update in place because muda registers its handler only once.
- Column width/reorder controls and toolbar controls displayed. Changing toolbar
  order applied immediately. The Preferences action footer remained visible with
  both customization sections expanded.
- The package retained native close/minimize/full-screen controls.

Native drag-out to another application's drop target is still unverified.

## CI portability fixes

The Ubuntu job in run `39081602830` failed Clippy on an archive mode cast: libc's
`st_mode` is u16 on macOS/Windows and u32 on Linux. The conversion now documents
that ABI difference and scopes the unnecessary-cast exception to that statement;
the workflow continues treating warnings as errors. The 0.4.0 Ubuntu job in run
`39092799534` reproduced the same failure.

The Windows job failed inside the Unix-only `openssh` dependency brought in by
OpenDAL's SFTP feature. SFTP is now enabled only for Unix targets, omitted from
the Windows service picker, and rejected explicitly when opening saved Windows
SFTP profiles. Windows defaults to WebDAV. Other remote adapters remain enabled.
A regression test covers platform service choices and the unsupported-profile
error path; the Windows branch still requires the next CI run.

Validation after these fixes:

- Formatting and Clippy with warnings denied pass on macOS.
- 114 tests pass on macOS; one interactive clipboard test remains ignored in
  normal runs and retains its earlier separate smoke-check evidence.
- Linux Clippy and all 114 tests pass with Rust 1.99.1 in an isolated ARM64
  Debian 12 container. The final source changes were checked again with Clippy.
- The `x86_64-pc-windows-msvc` Cargo dependency graph excludes `openssh` and
  `openssh-sftp-client`; a full Windows build/test run remains pending.
- Cargo.lock is unchanged and all checks use `--locked`.

## Rust Commander 0.5.0 — next twelve features

The [0.5 feature guide](../docs/FEATURES_0.5.md) records the delivered workflows
and their boundaries. All twelve feature areas have working implementations;
full upstream parity is still incomplete.

Validation on Apple Silicon macOS:

- Formatting, Clippy (`--all-targets -- -D warnings`) and all **140** normal tests
  pass with the locked dependency graph. The interactive clipboard and optional
  live SMB tests are excluded from the normal count.
- The optional live SMB test passes against a disposable Samba server bound only
  to loopback: list, mkdir, streamed upload/download, rename, guarded text save,
  filename/content search and recursive deletion.
- A separate signed test app used disposable files and isolated settings.
  Native checks confirmed persistent text preview, workspace saving,
  synchronization preview with a default Skip conflict, applying the one-sided
  copy, Undo and menu-driven Redo, journaled editor Save/Undo, and recovery usage
  and cleanup preview. Filesystem checks confirmed the saved workspace and the
  file changes. Quick Look launched Apple's `qlmanage`; its preview output was
  not visually validated. The utility's accessibility inspection timed out.
- Native testing caught shifted shortcut overlap in egui's logical modifier
  matching. Shift/Alt matching is now explicit; the regression test covers Redo
  and Search dispatch. New primary controls have German/Spanish translations.
- The release app is built and ad hoc signed. `codesign --verify --deep --strict`
  succeeds, and both the executable and bundle report **0.5.0**.

Validation on ARM64 Debian 12 in a disposable container:

- Clippy with warnings denied and all **140** normal tests pass. Linux's optional
  native GTK test is also ignored in the normal run.
- The live Samba workflow above passes from Linux as well as macOS.
- The GTK native drag-source test passes under Xvfb: the drag source advertises
  a native FileList with Copy, and its window is realized. Xvfb emitted a missing
  session-bus warning; actual cross-application drop completion and Wayland
  interaction remain unverified.

Additional regression coverage includes restart-safe history/Redo, external-edit
refusal, permissions/timestamp Undo, protected cleanup chains, interrupted queue
restoration without secrets, source-witness/prefix verification, checkpoint path
validation, synchronization staleness before/after recovery preparation,
TAR/7z mutation and Undo, encryption on every added ZIP file, remote guarded
saves/search, remote/archive drop routing, workspace round trips and persistent
locations for temporary archive tabs. Detected concurrent external changes
conservatively disable automatic Undo and retain recovery copies for review.

Remaining limits: no Windows machine was available; the Windows build and native
integration require CI/interactive validation. SFTP remains Unix-only. Live
cloud/OAuth flows and other remote servers are not established by the Samba
fixture. Remote saves cannot guarantee atomic external-edit protection on
services without conditional writes. Synchronization is local, and Linux
native drag-out uses a separate GTK drag window. Recovery targets can be exceeded
by protected chains. The parity completion gate still returns nonzero honestly.

Windows dependency setup now retries vcpkg installation at most three times.
This addresses the observed transient GitHub DNS download failure without masking
an exhausted failure or weakening warnings-denied checks.

## Dependabot GLib and Windows dependency setup

[Dependabot run 39102257724](https://github.com/ghillert/rust-commander/actions/runs/39102257724/job/117600759120)
failed because `drag` 2.1.1 pulled GTK3/GLib 0.18.5 into Cargo.lock; that series
cannot resolve to GLib 0.20+, which fixes RUSTSEC-2024-0429. The GTK3 backend was
unused: Rust Commander enables `drag` only on macOS/Windows and has a separate
GTK4 helper on Linux.

A [vendored macOS/Windows subset](../vendor/drag/README.md) removes the unused
backend and its dependencies. Native backend files are byte-for-byte identical
to upstream 2.1.1, and both upstream licenses are retained. The root NOTICE
includes the MIT notice for packaged binaries. Cargo regenerated the lockfile:
GLib 0.21.5 is the only remaining GLib version, 37 obsolete packages are gone,
and no unrelated package versions changed. No advisory is suppressed.
Target-specific package names/versions are unchanged on Apple Silicon macOS,
Windows MSVC and ARM64 Linux.

[Windows job 117600786532](https://github.com/ghillert/rust-commander/actions/runs/39102253000/job/117600786532)
failed before running Rust checks: vcpkg's liblzma download could not resolve
GitHub's hostname (curl error 6). The local workflow's three bounded attempts
now log each attempt and retry reason; an exhausted failure still fails CI.

Validation of the dependency fix:

- Linux Clippy with warnings denied and all 140 normal tests pass in an
  isolated ARM64 Debian 12 container with the updated locked dependency graph.
- macOS formatting, Clippy with warnings denied, and all 140 normal tests pass;
  the two desktop/live-service tests remain ignored in the normal suite.
- The macOS release bundle builds and its ad hoc signature verifies. Its bundled
  NOTICE includes the vendored dependency's MIT license.
- The Windows native drag crate passes `cargo check` for
  `x86_64-pc-windows-msvc` from the Mac, using the existing locked dependency
  versions. This does not link or run a Windows executable.
- The CI workflow YAML parses. Full Windows build/test execution and the
  Dependabot rerun require pushing the changes and running GitHub Actions.

## Slint migration (0.6.0)

The desktop frontend now uses Slint 1.18.1 with the existing Rust controllers,
workers and storage implementations. egui/eframe is absent from both the source
and the locked dependency graph. See the [Slint guide](../docs/SLINT_UI.md) for
architecture, styles and upgrade-sensitive native integration.

Automated checks:

- All **140** normal tests pass on Apple Silicon macOS. The interactive clipboard
  and live SMB tests remain ignored in normal runs; their earlier evidence above
  is historical, not a new live-service validation for this frontend.
- Formatting and warnings-denied Clippy pass with the locked graph.
- Slint callback tests cover editable paths, navigation, selection/filtering,
  copy/drop confirmation guards, invalid names/settings, remote/archive drops,
  active-right single-pane hit testing, shifted shortcuts, dirty-editor close
  protection, and keyboard focus after modal/terminal closure. Native modifier
  reconciliation is exercised through the actual Slint focus tree.
- Cupertino, Fluent and Cosmic styles compile on macOS. The optimized 0.6.0
  Mac bundle builds and its ad hoc signature verifies. Icons are embedded.
- ARM64 Debian 12 passes warnings-denied Clippy and all **140** normal tests,
  builds the screenshot-enabled executable, and produces a Cosmic client-area
  snapshot with the software renderer under Xvfb. Three optional tests are
  ignored on Linux, including its native GTK test.

Native macOS smoke checks used a separate bundle, disposable files and isolated
configuration:

| Workflow | Observed result |
| --- | --- |
| Native window | Close, minimize and full-screen/zoom controls remain in the accessibility tree. |
| Menus | AppKit command menus remain present after activation; menu-driven Undo and Settings work. |
| Copy and history | Confirmation selects the destination field; copying a Unicode filename and a text file completes. Menu Undo, Cmd+Z and Cmd+Shift+Z change the disposable destination as expected. |
| Address | Cmd+L selects the address; pasting a new absolute location and pressing Return navigates there. |
| Archives | A new ZIP appears through automatic refresh; opening it, entering an archive folder and viewing its text file work. |
| Appearance | Settings keeps its action footer visible; selecting Dark and applying it updates the interface and native window appearance. |
| Terminal | Opening focuses the PTY. A printf command displays its output, physical Control-C clears pending input, and output remains accessible by line. |

These checks caught and fixed menu replacement on window activation, transient
button models interrupting clicks, lost focus after dialog/terminal closure, and
macOS synthesized shortcut modifier delivery. Diagnostic key logging was removed.
Reviewing winit's native-drop paths also found omitted cursor coordinates.
AppKit, Win32 and X11 queries now identify the target pane; when a compositor
prohibits the query, the active pane's destination is displayed for confirmation.
Native cross-application drop completion remains unverified.

The initial Linux rendered launch exposed missing `libxkbcommon-x11`; it is now
an explicit README/CI dependency and the Xvfb launch passes. The container has
no desktop session bus, so it reports an xdg-settings watcher warning. This
check does not validate following a real desktop's light/dark preference.

Windows native execution, a real Ubuntu/GNOME/Wayland desktop, external native
file-drop completion and drag-out into another application's target remain
unverified. Native drag implementations are retained, with deferred dispatch
outside the controller borrow and guarded incoming-drop routing. Slint widget
styles are drawn controls with native window integration; they do not establish
exact AppKit/WinUI/Ubuntu widget parity. Live cloud/OAuth validation and the
remaining upstream parity items are still open.

## Desktop chrome refinement (0.6.0)

The main window now has compact icon controls, text breadcrumbs with an editable
path mode, sidebar glyphs and current-location highlighting, quiet tabs,
right-aligned sizes, a small active-pane marker and a slim shortcut bar. The
macOS toolbar uses a full-size content view with transparent native title bar;
space is reserved for the native window controls. Toolbar background dragging
uses winit's window-drag API; physical drag behavior remains unverified.

Light and dark client-area snapshots were rendered and visually reviewed on
macOS. A separate native review bundle used disposable files and isolated
settings. Accessibility inspection retained close, minimize and full-screen/zoom
controls, labeled toolbar buttons and editable filters. Native checks confirmed
Cmd+L path editing/navigation, filtering, copying through the new toolbar with
confirmation, and Cmd+Z Undo. The normal suite passes all **140** tests, including
actual keyboard entry through the compact address field after deferred focus
callbacks are processed; warnings-denied Clippy passes.

The refined chrome compiles with Cupertino, Fluent and Cosmic styles on macOS.
The updated optimized Mac bundle builds and its ad hoc signature verifies.
The earlier Linux container results above establish the migration baseline;
the refined chrome has not been exercised on a native Windows/Linux desktop.

The macOS traffic-light alignment was checked in an isolated native review
window. AppKit frame measurements place all three centers at 22 points in the
44-point toolbar, and at 20 points in the 40-point header with the toolbar
hidden. Native zoom resizing, entering/leaving full screen, and restoring the
toolbar retain the alignment. The standard controls remain accessible, and
the toolbar Settings action remains usable. The native title-bar hit area is
resized with the header to accommodate the controls. The four existing Slint UI
regression tests and Clippy with warnings denied pass after this adjustment.

## Windows checkpoint cleanup Clippy fix

[Windows job 117624736709](https://github.com/ghillert/rust-commander/actions/runs/39110262325/job/117624736709)
ran commit `5d2b57c` (before the Slint migration). Dependency installation
succeeded, but Clippy rejected `set_readonly(false)` in the Windows-only
checkpoint cleanup block. On Windows this clears `FILE_ATTRIBUTE_READONLY`;
the lint concerns the different Unix behavior. A statement-scoped exception
with an explicit reason preserves the Windows cleanup behavior and keeps the
workflow's warnings-denied policy enabled.

A standalone harness containing the exact cleanup function reproduces the
original error with Windows-target Clippy and accepts the fixed function.
This checks the reported lint; it does not link or run the Windows application.
macOS formatting, all-targets Clippy with warnings denied, and all seven queue
and resumable-transfer regression tests pass. Full Windows CI validation
requires a new run with the fix.

## Twelve parity additions (0.7.0)

The [0.7 guide](../docs/FEATURES_0.7.md) describes the new behavior and limits.
Implementation is compared with the existing pinned upstream source revision,
including the five-row block increment, XML keymap filename/alternate binding
attributes, nested visible preferences and upstream split-orientation terminology.
The checklist records these areas as partial where platform validation or exact
upstream compatibility remains incomplete.

Automated validation on Apple Silicon macOS:

- **166 normal tests** pass; the interactive clipboard and live SMB tests remain
  ignored. The 26 added tests comprise 16 integration cases and ten headless
  Slint/window-runtime cases.
- Formatting, warnings-denied all-targets/all-features Clippy and whitespace
  checks pass with the locked dependency graph.
- Coverage includes Unicode normalization/cycling, atomic wildcard selection,
  complete range marking, five-row blocks, locked/named tab navigation and
  persistence, layout/drop routing, quick lists and cross-pane navigation,
  custom command labels and bindings, metadata sorting, exact-byte encoded
  save/Undo, unrepresentable legacy characters, UTF-8/UTF-16 page boundaries in
  both directions, bounded recents and reviewed XML import with exact backups.
- Headless window-runtime tests retain independent tabs while sharing preferences,
  queue and history; closing another window does not cancel the job owner's work.
  The editor conversion callback test saves through a remapped keyboard binding.
- Trash tests exercise the confirmation guard and refusal to replay emptying
  unattended. They never call the native empty-Trash operation.
- Cupertino, Fluent and Cosmic styles compile on macOS. The optimized 0.7.0
  macOS bundle builds and its ad hoc signature verifies.
- The exact `src/native_actions.rs` module passes warnings-denied Windows-target
  Clippy in a standalone harness with the locked native dependency versions.
  This checks compilation of the new Recycle Bin API path; it does not link or
  run the Windows application.

Native macOS checks used a separate review bundle, configuration directory,
XML samples and files under `/private/tmp/rust-commander-07`:

| Workflow | Observed result |
| --- | --- |
| Quick find and selection | Typing a prefix selects the accented sample filename. Wildcard `*.txt` selects all five visible text files. |
| Multiple windows and recovery | Cmd+N opens another window. Undo there reverses the first window's disposable transfer through shared history. |
| Layout | Stacked panes render correctly with the common sidebar and independent location rows. |
| Quick lists | Filtering for Destination and pressing Return navigates there. |
| Tab locks | Locking the tab and choosing Parent leaves its original location locked and opens an unlocked parent tab. Window titles track the active tab and window number. |
| Settings | Named command-bar choices and shortcut filtering are accessible in the native window. |
| Text | A UTF-16 LE sample opens decoded and editable. Saving retains its `FF FE` BOM and CRLF. A 3.4 MB UTF-8 file opens read-only and advances from bytes 0–262144 to 262144–524288. |
| Import | Preview reports supported content and an unsupported preference. Applying adds the sample bookmark and reports a backup path in the isolated configuration. |

Windows and Linux desktop execution for 0.7 remains outstanding. Earlier Linux
container checks above establish the 0.6 baseline only. Real system Trash opening
and emptying, Finder automation prompts and GIO/GVfs integration have not been
executed in this validation. Native file-drop completion, live remote services,
additional encodings, full multiwindow session restoration, new-label translations
and the remaining upstream actions are still open. No full-parity claim is made.

## Windows nested archive paths

[Windows job 117743365140](https://github.com/ghillert/rust-commander/actions/runs/39150610371/job/117743365140)
checked out `941d7b9` (0.6.0), passed Clippy, then failed
`additional_archives_round_trip_nested_content` with
`Unsafe archive path: source\nested`. The macOS and Ubuntu jobs in the same run
completed successfully, including release packaging. This failure predates the
uncommitted 0.7 additions.

The archive walker was passing a locally assembled Windows `PathBuf` to the
strict raw-header validator before the writers could encode its separators.
Extraction, archive edits and synchronization also revalidated native paths in
the same way. A separate component-based conversion now emits forward-slash
relative names at those internal boundaries, preserving filename bytes and
rejecting roots, prefixes, parent traversal, NUL and colon components. Raw
archive headers continue to use the unchanged strict validator, including its
backslash rejection.

Validation of this fix:

- All **171 normal tests** pass on Apple Silicon macOS; two optional tests remain
  ignored. Formatting, whitespace checks and warnings-denied all-targets/
  all-features Clippy pass.
- New regression cases cover native joined paths, revalidation of parsed member
  paths, non-Unicode Unix filename preservation, nested ZIP/TAR/7z creation,
  selected extraction, nested add/rename/remove, and actual forward-slash headers.
- Raw ZIP, TAR and 7z fixtures still reject Windows backslashes, drive/UNC names,
  parent traversal and colon names before creating an extraction directory.
  Existing nested synchronization, conflict, recovery and archive tests pass.
- The exact path helpers and their Windows-specific test branches pass
  warnings-denied Clippy for `x86_64-pc-windows-msvc` in a standalone harness.
  This is a compile check, not Windows test execution. A new Windows CI run with
  the fix is still required to confirm the full suite there.

## Windows ZIP replacement handle lifetime

[Windows job 117760776917](https://github.com/ghillert/rust-commander/actions/runs/39156505054/job/117760776917)
checked out `2f88c64`, passed Clippy and all 39 Windows library tests, including
the native path and nested synchronization regressions. The nested archive test
now passes ZIP creation, listing and selected extraction, then fails on its
first ZIP edit with `failed to persist temporary file: Access is denied.
(os error 5)`. The macOS and Ubuntu jobs complete successfully, including release
packaging.

The ZIP editor retained its input `ZipArchive<File>` until the function returned,
so the destination was still open when `NamedTempFile::persist` called
`MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)`. The editor now explicitly drops
that reader after copying and validating all entries and before its final
fingerprint check and atomic replacement. The original-change guard, permission
preservation and staged write remain in place; replacement errors do not delete
the original to work around a lock.

Validation of this fix:

- All **47** targeted archive, recovery and feature integration tests pass on
  Apple Silicon macOS; the two optional clipboard/live SMB cases remain ignored.
  Formatting, whitespace checks and warnings-denied all-targets/all-features
  Clippy pass.
- An isolated x86-64 Win32 executable run under Wine 11.0 on the Mac reproduces
  error 5 when `MoveFileExW` replaces a destination with its reader still open,
  even with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`. Closing the
  reader makes the same replacement succeed. The diagnostic uses disposable
  files and its own Wine prefix; this is emulated Win32 evidence, not a native
  Windows application test.
- A Windows-only regression keeps an external reader open, checks that an edit
  fails without changing the original bytes, then closes the reader and verifies
  the same edit succeeds with both original and added entries intact. The
  existing nested ZIP/TAR/7z edit test also covers the reported failure. The new
  Windows-specific case still requires execution by Windows CI.


## Windows timestamp metadata access

[Windows job 117775997124](https://github.com/ghillert/rust-commander/actions/runs/39161594318/job/117775997124)
checked out `a861da4`, passed Clippy, all 39 library tests and all three archive
path tests, including the external-reader replacement regression. It then failed
`timestamp_changes_are_reversible` on the initial timestamp change with
`Access is denied. (os error 5)`. The macOS and Ubuntu jobs succeeded, including
release packaging. This run confirms the preceding archive fixes on Windows.

`desktop::change_modified` opened a read-only handle before calling `set_times`.
Windows [SetFileTime requires FILE_WRITE_ATTRIBUTES](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfiletime).
The Windows branch now requests that metadata access directly through
`OpenOptionsExt::access_mode`, with `FILE_FLAG_BACKUP_SEMANTICS` to support
folders. It does not request file-content write access or clear read-only
permissions. Symlink refusal remains in place.

Validation of this fix:

- All **17** active `features_05` integration tests pass on Apple Silicon macOS;
  the disposable live SMB test remains ignored. Timestamp regressions check
  file and folder changes through Undo/Redo, unchanged contents and child
  timestamps, read-only permission preservation and refusal to follow symlinks.
- Formatting, whitespace checks and warnings-denied all-targets/all-features
  Clippy pass. The exact timestamp function also passes warnings-denied Clippy
  for `x86_64-pc-windows-msvc` in a standalone harness. This checks the Windows
  branch without claiming a full Windows application build or test execution.
- A disposable Win32 diagnostic under Wine 11.0 can set and read back file and
  directory timestamps with the new metadata-only access. However, Wine also
  accepts the old read-only handle and does not expose the expected read-only
  attribute for its fixture. This diagnostic therefore does not validate native
  Windows access enforcement or read-only behavior.
- The new Windows-specific behavior and recovery regressions still require a
  native Windows CI run with this fix.

## Release size and dependency features

On Apple Silicon macOS with Rust/Cargo 1.99.1, the unchanged release at `96c97d7`
was rebuilt at **40,163,792 bytes**. The selected `opt-level = "z"`, full LTO,
single-codegen-unit profile produces **19,714,944 bytes**, a **50.9%** reduction.
Panic unwinding, both renderers, accessibility, PDF/image formats, archives and
remote services remain enabled. The dependency audit removes only unused
OpenDAL in-memory storage and clipboard image-transfer features; package versions
are unchanged.

All **174 ordinary tests** pass in debug and optimized release; formatting and
warnings-denied all-targets/all-features Clippy pass. The separate native clipboard
round-trip passes and restores readable PNG/TIFF data. Both rendering backends
open the selected executable and navigate the two-page PDF fixture. GPU text/image
viewing, CLI contracts and strict macOS bundle signature verification also pass.
Windows file/text clipboard APIs compile with the audited flags. Windows/Linux
native release builds and sizes, live remote services and broader workload
performance remain unverified in this run.

See [the release size guide](../docs/RELEASE_SIZE.md) for the profile comparison,
exact measurements, feature audit, reproduction steps and compiler tradeoffs.


## Next twelve feature workflows (2026-10-19)

The [next-feature guide](../docs/FEATURES_NEXT.md) describes the twelve additions,
entry points and limits. They extend the Slint app while retaining Hayro and the
optimized release profile. This validation does not establish full muCommander
parity.

- `test --locked --all-targets --all-features`: **197 passed**, no failures;
  the optional native clipboard and live disposable SMB cases remain ignored.
  The batch adds 18 backend regressions and five desktop/window callback tests.
- `test --release --locked --test features_08`: **18 passed**, including native
  move identity/Undo, archive merge/save-back, interrupted checkpoint recovery,
  stale-source/partial-file refusal, remote merges/sync, legacy history records,
  encrypted PDFs and macOS metadata. All **45 library tests** also pass after
  the final PDF scroll-layout correction.
- Formatting, whitespace checks and warnings-denied all-targets/all-features
  Clippy pass. The Windows native metadata implementation passes standalone
  warnings-denied Clippy for `x86_64-pc-windows-msvc`; this is a compile check,
  not Windows execution or a complete Windows application build.
- Native Apple Silicon macOS interaction uses a disposable application bundle,
  configuration and files. Directory expansion and Down/Enter tree navigation,
  text search at byte 22, hexadecimal byte jump to 16, wrong/correct PDF
  passwords, direct page-two navigation, rotation, fit-width and vertical
  scrolling pass. Quitting and restarting restores two windows at distinct
  locations, both reachable through the Windows list.
- Software and femtovg snapshots exercise the tree/PDF layout. Active-folder
  reveal uses a fixed tree content height; an explicit PDF scroll-content size
  fixes the clipped/blank rotated fit-width page found during native testing.
- `scripts/package-macos.sh` completes and the rebuilt bundle passes
  `codesign --verify --deep --strict`. The unbundled optimized executable is
  **19,997,968 bytes**; this later feature build is separate from the preceding
  release-profile-only comparison.

The parity gate still correctly reports **4 implemented, 1 inapplicable,
159 partial and 36 missing** actions/modules, returning nonzero. Windows/Linux
native behavior, live remote authentication/services, cross-volume move fallback
and provider-specific upload append/conditional-write semantics need dedicated
platform/server runs. Filesystem-backed remote fixtures exercise the common
transfer/sync/checkpoint code; they do not substitute for those protocol runs.
Checkpoint copies currently require unused targets and do not offer local Undo;
remote archive containers remain read-only. See the feature guide for the other
bounded-input, metadata and recovery limits.


## Stabilization pass (2026-10-19)

See [the stabilization guide](../docs/STABILIZATION.md) for commands, fixture
isolation, coverage and cleanup. The existing Windows/macOS/Ubuntu CI matrix now
checks all features, adds optimized recovery tests on every OS, and runs real
protocol services, native clipboard/GTK drag checks and two-renderer snapshots on
Ubuntu. This records local evidence and configured CI coverage; the changed
workflow has not yet executed on GitHub.

Local Apple Silicon macOS results:

- **207 ordinary tests pass**, including actual Slint accessibility actions,
  focus routing/modal traversal, symlink-alias/hidden-ancestor tree following,
  display-independent CLI contracts, file-backed upload buffers, subprocess
  termination/retry, interrupted replacement and overlapping remote deletions.
  Seven cases are intentionally ignored by the ordinary command; the parent
  recovery test invokes its ignored crash helper explicitly.
- All **four real protocol suites pass** against disposable Linux containers:
  OpenSSH/SFTP, pyftpdlib FTP, Samba SMB2/3 and WsgiDAV. Each covers listing,
  Unicode paths, upload/download, range reads, recursive merge conflicts,
  cancelled download/retry, checkpoint publication, existing-target refusal,
  synchronization/stale plans, remote copies, rename and deletion. Password
  rejection is checked for FTP/WebDAV/SMB; SFTP uses an ephemeral key and a pinned
  host key with strict verification. No user accounts/profiles/SSH files are used.
  The harness removes its containers and private temporary configuration.
- **21 optimized feature/recovery tests pass** (18 `features_08`, three
  `stabilization`), with the crash helper invoked only by its parent.
- Formatting, whitespace checks and warnings-denied all-target/all-feature
  Clippy pass. CI YAML and Python syntax parse successfully.
- Software and femtovg both render the file view and rotated PDF: all four PNGs
  are 2480×1560. Visual review verifies the active aliased folder is revealed;
  the final GPU tree diagnostic selects row 459 and scrolls to -12203. This is
  a smoke check, not a pixel-perfect visual assertion.
- The native Mac file clipboard copy/cut/external-change round-trip passes.
  macOS emits sandbox-extension and legacy HTML pasteboard-type diagnostics;
  this does not establish exact preservation of every private clipboard format.
- The final macOS package builds and passes strict/deep signature verification.
  The unbundled release executable is **19,998,032 bytes**. The existing size
  profile is retained; bytes/memmap2 add direct edges to already-locked packages,
  without adding or upgrading package versions.

The live tests exposed and now guard two faults: FTP bulk deletion left nonempty
parents behind while reporting success, and repeated writes through a
single-request WebDAV upload adapter could spin instead of completing a large
upload. Deletion now proceeds from children to parents and verifies absence;
single-request uploads use an owned mapping of a private temporary file. Staging
is cancellable and requires temporary disk space; provider commit calls retain
cooperative cancellation/I/O timeout limits.

Native Windows/Linux execution of the updated application and CI steps remains
pending, including the protected Windows DACL regression and Linux Xvfb checks.
Cloud OAuth/accounts, S3/GCS and other server implementations, full screen-reader
interaction and cross-volume failure scenarios still need dedicated validation.
The parity gate remains **4 implemented, 1 inapplicable, 159 partial, 36 missing**
and correctly returns nonzero.


## Direct URI navigation (2026-10-21)

Implemented editable addresses by default, a persistent breadcrumb toggle,
canonical protocol labels, explicit saving of named connections, HTTP directory
indexes/downloads, both FTPS styles, and Windows/password SFTP through a Rust
transport. Existing Unix OpenSSH key/agent connections remain available.

Checks actually performed on the available Apple Silicon Mac:

- `cargo test --locked --all-targets --all-features`: 236 passed, no failures;
  15 environment-dependent tests were ignored in that invocation.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` and
  `cargo fmt --all -- --check` pass.
- `scripts/test-uri-transports.py`: six real loopback workflows pass, covering
  plain/anonymous FTP, explicit and implicit FTPS, Rust SFTP passwords and keys,
  certificate/host trust rejection, and missing-root rejection without creation.
  The writable protocols additionally exercise Unicode names, range reads,
  streaming, guarded saves, conflicts, cancellation/resume, checkpoint publishing,
  synchronization/stale plans, rename and deletion.
- HTTP fixtures cover directory redirects, encoded and query-bearing child
  links, unknown metadata, direct-file copying, range fallback, ordinary-webpage
  rejection, response limits, authentication errors and missing resources.
- UI/controller checks cover Apply/Cancel, default/legacy preferences, unnamed
  descriptors and explicit saving, restoring tab references, read-only actions,
  and stale results after navigation or pane/tab switching.
- Eight disposable native app windows rendered URI listings successfully:
  software and FemtoVG, address and breadcrumb modes, light at 1× and dark at 2×.
  Representative screenshots were visually inspected. Outputs are under
  `target/uri-smoke/` and are not committed.
- The supplied Kew HTTPS directory passed the live read-only integration check
  and a desktop check showing its two folders and two ZIP files. No ZIP was
  downloaded for this acceptance check.
- Icon catalog: nine tests pass, 85 icons and 88 pinned assets/notices verified.
  Release automation: 18 tests ran successfully, with two Linux-only cases skipped.

The portable transport harness is wired into Windows, macOS and Linux CI. Linux's
existing disposable OpenSSH harness also exercises the new Rust SFTP transport.
Windows OpenSSH-agent integration, native Windows/Linux desktop interaction,
private cloud accounts and their credentials have not been exercised on this Mac.
These are configured checks or remaining environment limits, not claimed results.

The release size comparison uses the unchanged ARM64 macOS architecture and the
same optimized Cargo release profile, with no screenshot feature. Baseline before
this change: 20,314,528 bytes (19.37 MiB). Final size: 22,007,008 bytes (20.99 MiB), an increase of
1,692,480 bytes (8.33%). The superseded OpenDAL FTP feature was removed; FTP/FTPS
use the direct SuppaFTP adapter without its unused asynchronous OpenDAL transport.


## HTTP directory metadata — 2026-10-21

HTTP indexes now map server-provided sizes and modification times into the
shared entry metadata used by the file-list columns. Coverage includes Apache
preformatted/table indexes, nginx exact/human sizes, Caddy exact byte counts and
ISO timestamps, zero-byte files, missing/invalid values, and row boundaries.
Rounded index sizes remain display metadata; transfers still use the real stream
and authoritative resource metadata.

On the Apple Silicon Mac, 242 tests passed (15 environment-dependent cases
ignored), Clippy with warnings denied passed, and formatting/diff checks passed.
The live read-only Kew test confirmed sizes for both ZIPs and modification times
for all four entries without downloading ZIP files. Light/software and
dark/FemtoVG screenshots were visually inspected and show populated columns;
outputs are in `target/uri-smoke/kew-metadata-*.png`. Windows/Linux checks were
not run locally for this fix. Index timestamps without explicit zones use UTC
and are rendered in the user's local timezone.


## Transfer speed — 2026-10-21

Active operations show a recent byte rate in a separate status-bar label, which
stays visible when a long file path is elided. Rates use a bounded two-second
window, fall to zero during stalls, and exclude restored checkpoint byte counts.
Transient timing samples are not serialized.

On the Apple Silicon Mac, 247 tests passed (15 environment-dependent cases
ignored), and Clippy with warnings denied, formatting, and diff checks passed.
Deterministic checks cover steady throughput, idle decay, restarting after a
stall, rapid chunk updates/cache bounds, setup time, restored offsets and
serialization. A Slint testing-backend test verifies that progress shows both
byte count and speed, then clears the speed when the job ends. These checks are
automated; no native Windows/Linux runtime check or network benchmark was run
for this change.


## Determinate transfer progress — 2026-10-21

The progress bar now uses a whole-operation byte budget for local and remote
copies/moves, conflict-resolved transfers and checkpoint recovery. Totals use
fresh metadata, exclude skipped files, and include recovery/staging copy work.
Verified checkpoint offsets and native renames advance progress without
inflating transfer speed. Unknown or changing lengths use an indeterminate bar;
active jobs reserve 100% until flushing, publishing and verification finish.
Other operations without a byte budget remain indeterminate.

On the available Apple Silicon Mac, 251 tests passed (15 environment-dependent
cases ignored), plus six real disposable FTP, explicit/implicit FTPS and Rust
SFTP workflows. Clippy with warnings denied, formatting and diff checks passed.
Tests cover rounded HTTP listing sizes versus exact HEAD metadata, streaming
without Content-Length, local folders/native moves, skipped/replaced files,
cancellation and verified partial-file recovery. HTTP reads without a known
length now consume the existing timeout-layered accessor stream through EOF.

A real throttled 32 MiB loopback download rendered a 31% filled bar and live speed
in light/software and dark/FemtoVG native windows. Both captures were visually
inspected (`target/progress-smoke/`); the fixture and automated checks are in
`scripts/progress-smoke.py`. Linux CI is configured to run this under Xvfb.
Native Windows/Linux desktop validation was not performed on this Mac.


## Drag and drop — 2026-10-21

Local, archive and remote selections now share typed, stable payloads and guarded
Copy/Move confirmation. Targets include panes, folders, tabs, breadcrumbs, sidebar
locations, directory-tree nodes and writable local archives, with hover navigation
and file-list edge scrolling. Starting or abandoning a gesture never downloads or
extracts virtual contents. External virtual exports require the explicit
**Prepare files for external drag…** command; lazy native file promises remain a
gap. Archive conflict previews inspect metadata without exporting payloads.

Checks actually performed on the available Apple Silicon Mac:

- Full application tests: 270 passed, no failures, 15 environment-dependent
  cases ignored. Clippy with all targets/features and warnings denied passed.
  Formatting and diff checks passed.
- Automated cases cover frozen multi-selections, destination routing, cancellation,
  stale payloads, preparation lifecycle, read-only targets, exact pointer release
  routing, archive/local/remote transfer combinations, skipped Move sources,
  recursive targets, durable archive recovery, local Undo and metadata-only
  conflict discovery. A loopback HTTP regression confirms zero content requests
  when a virtual drag is abandoned. Explicit preparation completion cannot launch
  an unrelated mouse gesture.
- Physical desktop checks: local Move, Undo, folder-row targeting and Cancel;
  archive-entry Copy into a local folder; and the live Kew HTTPS listing. Dragging
  its ZIP onto an invalid target started no transfer. Dropping it into the other
  pane opened Copy confirmation without downloading; Cancel left it unchanged.
  The isolated test profile had no export folder or downloaded Kew ZIP afterward.
- File/PDF desktop smoke captures passed with software and FemtoVG renderers
  (`target/desktop-smoke/`). The FemtoVG file-list capture was visually inspected.
  The updated debug app bundle built and passed ad hoc signature verification.
- Icon catalog checks passed (nine tests); release-tooling checks passed
  (18 tests, including two platform-dependent skips).

Linux checks ran in an ARM64 Debian Bookworm container with Rust 1.92: application
check and all-target/all-feature Clippy passed. Native fixtures passed under Xvfb
(two XDND cases, including Unicode/large INCR payloads and cancellation), plus
five Wayland URI/pipe checks and one headless Weston connection/lifetime check.
The full Linux application test executable exceeded this container's link memory
limit; its full test suite is not claimed as passed. Logs are retained under
`target/native-dnd-logs/`. CI now includes the native fixture command.

The changed Windows native helper and vendored OLE source cross-compiled for
x64 MSVC. A full Windows application build and interactive desktop checks remain
for CI/Windows. Finder, Explorer, Nautilus and Dolphin interoperability, physical
Wayland gestures, and every display scale have not been acceptance-tested here.
Linux outgoing native file lists and all virtual exports currently advertise Copy;
use the application's guarded Move confirmation between its panes/windows.
See [drag-and-drop documentation](../docs/DRAG_AND_DROP.md) for export retention
and backend limitations.


## Deferred native exports and drag hardening — 2026-10-22

Native source offers now defer virtual-file contents until the accepted-drop path:
AppKit file promises, Windows Shell descriptors/streams, and X11/Wayland URI
providers. Descriptor/name probes do not download or extract file bodies. Linux
local-file sources negotiate Copy/Move; virtual exports remain Copy. Preparation
remains a compatibility fallback for receivers requiring pre-existing file URLs.

Deferred sessions share one staged export across concurrent requests, preserve
originals, reject unsafe descriptor paths, report errors, support cancellation,
and time out idle receivers. AppKit destinations publish complete files/folders
without overwriting an existing path. Leases protect active exports during cache
cleanup; expired interrupted exports can be recovered by cleanup. Native handoff
handles window-exit events and polls native pointer positions while held, rather
than authorizing work from a released gesture.

Checks actually completed:

- Apple Silicon Mac: the full application suite passed 278 tests, with no failures
  and 15 environment-dependent skips. After the final lint/status cleanup, the
  affected 17 drag tests passed again. All-target/all-feature Clippy with warnings
  denied, formatting, and patch whitespace checks passed.
- New lifecycle cases include abandoned offers, accepted concurrent readers,
  corrupt exports, cancellation after materialization, idle receiver timeouts,
  released gestures, exact folder/file delivery, destination collisions, and
  active/unowned directory protection during interrupted-export cleanup.
- The actual AppKit provider fixture passed on the main thread: native name
  queries perform no writes; the deferred delegate writes the requested file and
  completes its callback; abandoning a provider does not invoke the writer.
  This is provider validation, not a Finder drag-and-drop acceptance result.
- Linux ARM64 Debian/Rust 1.92: application Clippy passed. Three Xvfb XDND tests
  passed, including early content probes, accepted/canceled deferred delivery,
  native Move negotiation, Unicode and large INCR lists. Six Wayland pipe/URI
  cases and the headless Weston connection/lifetime test passed. A complete Linux
  application test run is still not claimed; the earlier link-memory limit remains.
- Windows x64 MSVC: the native adapter and OLE provider fixture cross-compiled.
  The fixture checks descriptor access on hover, rejected early content reads,
  accepted-drop gating and cancellation. It was not executed on Windows locally.
- Nine icon catalog tests passed. Release tooling ran 18 tests with two
  platform-dependent skips. The updated macOS development bundle built and passed
  its ad hoc signature verification.

Desktop smoke checks used an isolated profile and a loopback HTTP directory. A
real in-window gesture opened Copy confirmation; Cancel and abandoned external
gestures produced no GET of the file body or export folder. Attempts at a Finder
cross-application gesture did not establish an accepted drop, so successful Finder
delivery is **not** claimed. Explorer/Nautilus/Dolphin interaction, physical
Wayland drags, Windows responsiveness with large exports, and cold virtual drags
between separate Rust Commander windows remain acceptance/compatibility gaps.
For receivers without virtual-file support, prepare the selection first.

GitHub CI now runs AppKit/OLE provider fixtures on macOS/Windows and the native
protocol suite on Linux. These workflow changes have not been pushed or executed
on GitHub; the results above are local Mac, cross-compilation and container runs.
Logs are retained in `target/native-dnd-logs/promises-*.log`; no remote CI success
is inferred from them. The parity overview remains **partial** for drag and drop.

## Cross-window drops, Windows responsiveness, and remote comparison — 2026-10-22

The current changes add private same-process selection handoffs on AppKit, OLE,
X11, and Wayland. The receiving window opens guarded transfer confirmation
without preparing, downloading, or extracting an external export. Tokens expire,
are single-use, and reject cancelled/completed sessions. Wayland latches private
delivery so a late URI request cannot start an export. macOS accepts modern file
URLs, preserving legacy incoming handling. Fast window-exit gestures preserve a
real pressed row selection; physical button state prevents delayed handoffs after
release.

Windows virtual files expose `IDataObjectAsyncCapability`; completion can arrive
before or after the native drag loop returns. Synchronous receivers export on a
worker while COM dispatches window messages/calls. New fixtures cover completion
ordering, failure, and delivery of a Win32 window message during a blocked export.
They have been cross-compiled here, **not executed on Windows**.

Folder comparison now recursively hashes local/local, local/remote, and
remote/remote file streams. Tests cover equal/different contents, one-sided files,
Unicode nested paths, and an actual loopback HTTP listing whose rounded size does
not equal the content length. Sync retains typed pane locations, cancels obsolete
previews, and filters read-only destinations. Execution rejects invalid choices
before any write, including restored queue plans. Local-backed remote operators
exercise reviewed mixed/remote sync; these are not live cloud-service acceptance.

Observed failures in [CI run 39838462721](https://github.com/ghillert/rust-commander/actions/runs/39838462721)
were addressed: Windows native joined paths use portable path validation, lease
cleanup reads the already locked file handle, and pinned cargo-packager setup
handles an executable restored by another cache. CI now runs checksum-verified
actionlint 1.7.12 over all workflows. Local actionlint accepts the current workflows
and rejects the original malformed release workflow. Composite-action YAML also
parses successfully. These working-tree changes have not been pushed; no new
GitHub CI success is claimed.

Automated checks on this Mac and a disposable ARM64 Linux container:

- macOS Apple Silicon: `cargo test --locked --all-targets --all-features`:
  **284 passed, 0 failed, 15 ignored**. Formatting and warnings-denied all-targets,
  all-features Clippy passed. A screenshot-enabled development binary built.
- The actual main-thread AppKit provider fixture passed metadata-only hover,
  deferred write, completion, and abandonment checks.
- Windows x64 MSVC: the native receiver, provider, and responsiveness fixture
  pass `cargo check --locked -p rust-commander-native-dnd-tests --tests
  --target x86_64-pc-windows-msvc`.
- Linux Rust 1.92: warnings-denied all-targets/all-features Clippy passed.
  **3 actual XDND fixtures** passed under Xvfb, including a private internal
  target that never invokes the export provider. **7 Wayland URI/pipe tests**
  passed, and **1 live headless Weston connection/lifetime test** passed.
- Release automation: **18 tests, 2 platform skips**. Icon catalog: **9 passed**.
  Workflow validation, formatting, and `git diff --check` passed.

Physical Mac acceptance used an isolated Rust Commander QA bundle, disposable
configuration, loopback HTTP server, ZIP, and destination. The user performed
Finder drops because the automation tool did not deliver a held drag outside the
source window. Exact byte checks confirmed local-file delivery (Finder negotiated
a move), a deferred HTTP file, a nested HTTP folder, and an archive entry.
Remote sources and the original ZIP remained intact. A cold archive drag from
QA Window 2 into QA Window 1 opened confirmation with the original archive
location and the correct destination. Before confirmation, the destination was
empty and the export-cache inventory was unchanged. Confirming copied the exact
bytes, preserved the original ZIP, and still created no native export. Earlier
automated in-window gestures opened normal confirmation and cancellation left
the destination empty.

The Linux full application suite was not rerun in the container. Windows execution,
Explorer/Nautilus/Dolphin interoperability, real Wayland desktop gestures, and
large-transfer responsiveness still need native acceptance. Protocol fixtures do
not establish those desktop contracts. Logs are retained under
`target/native-dnd-logs/next-*.log` and `rc-next-final-*.log`.

## Advanced search and folder tools — 2026-10-23

The local implementation adds shared local/remote streaming content search with
regex/case/size/date/exclusion controls, comparison filters and presets, explicitly
unverified quick metadata comparison, bounded read-only text differences, and
filtered synchronization that plans missing folders and files separately.

Validation on Apple Silicon macOS:

- `cargo test --locked --all-targets --all-features`: **299 passed, 0 failed,
  15 ignored**. The final layout adjustment was followed by rerunning both new
  Slint callback tests successfully.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`, formatting
  and `git diff --check` passed. The screenshot-enabled development binary built.
- Nine domain regressions cover searches beyond the old file-size cutoffs,
  Unicode case matching across buffers, regex/date/size/exclusion filters,
  cancellation and visible long-line errors, metadata comparisons with unknown
  timestamps, all four local/filesystem-adapter synchronization combinations,
  excluded descendants, stale/type-conflict refusal, preset serialization and
  aligned local/remote text differences.
- Actual Slint callbacks cover invalid regex input, background search, preset
  save/load across comparison and synchronization, invalidation of reviewed plans,
  and opening/closing a read-only side-by-side diff.
- Search, populated comparison and text-diff windows rendered successfully with
  both software and femtovg renderers. Screenshot inspection found and corrected
  stretched controls/rows; the final forms and diff were inspected again.

Logs are in `/private/tmp/rc-advanced-*.log`; six final screenshots and renderer
logs are in `/private/tmp/rc-advanced-visual/renders/`. All rendered runs used
disposable files and isolated configuration. These are local checks, not a claim
of CI success for the uncommitted feature changes. Native Windows/Linux interaction
and broader live-provider acceptance remain open.

## Transfer controls and durable conflict recovery — 2026-10-23

Validation on Apple Silicon macOS:

- The complete all-target/all-feature suite passed: **314 passed, 0 failed,
  16 ignored**. Formatting, all-target/all-feature Clippy with warnings denied,
  and `git diff --check` passed.
- Shared-controller tests cover acknowledged pause, cancellation while paused or
  backing off, live bandwidth changes, bounded temporary-error retries, permanent
  failures and credential redaction. Slint callbacks exercise pause/resume through
  the queue dialog, live settings validation and checkpointed local folder copies.
- Durable conflict tests cover all four local/filesystem-adapter directions,
  fixed Keep both names, per-file overrides, retained originals, stale destination
  and corrupt-backup refusal, source aliases, metadata preservation, safe discard
  and exclusive checkpoint leases. Worker subprocesses terminate during backup,
  deletion and publication before the completion receipt; fresh workers recover.
- The portable URI harness passed all **six** real loopback fixtures: anonymous
  FTP, authenticated FTP, explicit FTPS, implicit FTPS and native SFTP with password
  or public-key authentication. Each now also exercises checkpointed whole-folder
  replacement and merged file replacement.
- Paused transfers and the queue controls rendered with both software and femtovg.
  Visual inspection caught stretched settings fields; the corrected compact queue
  form was rendered and inspected again.

Logs are under `/private/tmp/rc-transfer-*.log`; final screenshots are under
`/private/tmp/rc-transfer-visual-final/`. Tests and screenshots used disposable
configuration and files. These checks do not claim native Windows/Linux acceptance,
cloud-provider coverage or CI success for this uncommitted batch.
