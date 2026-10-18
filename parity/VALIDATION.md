# Validation record

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
