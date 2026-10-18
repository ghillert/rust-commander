# Validation record

This record covers version 0.4.0 on the available Apple Silicon Mac, including
Linux checks in an ARM64 Debian container. It is not a
full muCommander compatibility certification. See [PARITY.md](PARITY.md) for
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

## Validation limits

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
