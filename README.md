# Rust Commander

[![CI](https://github.com/ghillert/rust-commander/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ghillert/rust-commander/actions/workflows/ci.yml)
[![Rust 1.92+](https://img.shields.io/badge/Rust-1.92%2B-orange?logo=rust)](Cargo.toml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)

A dual-pane desktop file manager written in Rust, inspired by
[muCommander](https://github.com/mucommander/mucommander). Browse local files and
archives, manage transfers, and work in an embedded terminal from one window.
The interface uses [Slint](https://slint.dev) and runs without a JVM or browser engine.

Rust Commander is an independent project and is not affiliated with or endorsed
by the Rust Project or Rust Foundation.

> **Development preview — 0.6.0.** This is an independent implementation with
> full upstream compatibility still in progress. Apple Silicon macOS has been
> validated interactively; Windows/Linux desktop integration still needs testing.
> See the [parity checklist](parity/PARITY.md) and
> [validation record](parity/VALIDATION.md) for the current scope.

## Contents

- [Features](#features)
- [Getting started](#getting-started)
- [Usage](#usage)
- [Desktop appearance](#desktop-appearance)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Remote connections](#remote-connections)
- [Configuration](#configuration)
- [Development](#development)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

## Features

- **Dual-pane navigation:** tabs, history, editable locations, bookmarks,
  filtering, sorting, hidden files, and single-pane mode.
- **Local file operations:** copy, move, rename, create, Trash, and permanent
  deletion, with skip, keep-both, recoverable replacement, and folder merging.
- **Archive support:** create, browse, and extract ZIP, TAR variants, 7z, and
  compressed streams; edit local ZIP/TAR/7z containers, create/unlock encrypted ZIP and 7z,
  and browse nested or remote archives. Read additional formats through libarchive.
- **Recovery and transfers:** guarded local Undo/Redo, storage retention controls,
  persistent history/queue, verified local file resume, and per-file conflict choices.
- **Desktop integration:** system file clipboard, mounted drives, clickable
  breadcrumbs, native file actions, configurable columns/toolbar, and native
  drag-out (a GTK helper window on Linux).
- **Languages:** system language, English, German, and Spanish.
- **File tools:** batch rename with preview, recursive name/content search,
  folder comparison, checksums, properties, permissions, and split/combine.
- **Built-in viewers:** local/remote UTF-8 editing, images, hex, PDF page navigation,
  a persistent preview pane, and macOS Quick Look.
- **Synchronization and workspaces:** reviewable local folder synchronization and
  named sets of pane locations, tabs and layout.
- **Embedded terminal:** a PTY-backed shell in the active local folder; hiding
  the terminal preserves its session.
- **Desktop appearance:** macOS, Windows, Ubuntu, and GNOME profiles, installed
  system fonts, light/dark mode, and native macOS menus and window controls.
- **Remote adapters:** saved connection profiles and streaming transfers for
  SMB2/3, SFTP, FTP, WebDAV, S3, and several cloud services. Validation is limited; see
  [Remote connections](#remote-connections).

See the [Slint desktop guide](docs/SLINT_UI.md) for the 0.6 interface,
the [0.5 twelve-feature guide](docs/FEATURES_0.5.md) for usage and limits,
the [0.4 guide](docs/FEATURES_0.4.md) for earlier workflows, and
[file operations and format support](docs/FILE_OPERATIONS.md) for recovery and
archive details. Windows/Linux desktop interactions and live cloud services
still need validation.

## Getting started

### Prerequisites

- Rust **1.92 or newer**, including Cargo.
- Native build tools for your operating system.
- libarchive, with `pkg-config` on macOS/Linux or vcpkg on Windows.

<details>
<summary>macOS</summary>

Install the Xcode Command Line Tools if needed:

```sh
xcode-select --install
```

Install `pkg-config`, for example with Homebrew:

```sh
brew install pkg-config
```

The project helper script can use macOS's system libarchive. A Homebrew
libarchive installation also works.

</details>

<details>
<summary>Ubuntu / Debian</summary>

Install the native dependencies:

```sh
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config libarchive-dev libdbus-1-dev libgtk-4-dev libfontconfig1-dev \
  libx11-dev libxfixes-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev \
  libgl1-mesa-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
```

Other distributions need the equivalent development packages.

</details>

<details>
<summary>Windows</summary>

Install the Visual Studio C++ build tools and vcpkg, and use Rust's MSVC
toolchain. From a PowerShell session with the build tools available, set the
path to your vcpkg checkout and install libarchive:

```powershell
$env:VCPKG_ROOT = "C:\path\to\vcpkg"
$env:VCPKGRS_TRIPLET = "x64-windows-static-md"
& "$env:VCPKG_ROOT\vcpkg.exe" install libarchive:x64-windows-static-md
```

Keep these environment variables set when running Cargo. Windows setup is
configured in CI but has not yet been validated on a Windows machine.

</details>

### Build and run

Clone the repository:

```sh
git clone https://github.com/ghillert/rust-commander.git
cd rust-commander
```

On macOS or Linux:

```sh
./scripts/cargo-local.sh run --locked --release
```

On Windows:

```powershell
cargo run --locked --release
```

The helper script uses a project-local Rust toolchain in `.tools/` when one is
present; otherwise it uses your installed toolchain. A fresh clone does not
include a toolchain or build output.

### Package the macOS app

```sh
./scripts/package-macos.sh
open "target/release/Rust Commander.app"
```

The bundle uses an ad hoc signature. Notarization and release installers are
not configured.

## Usage

The executable accepts up to two starting directories:

```text
rust-commander [LEFT_DIRECTORY] [RIGHT_DIRECTORY]
```

Pass them after `--` when launching through Cargo. For example, on macOS/Linux:

```sh
./scripts/cargo-local.sh run --locked --release -- /path/to/left /path/to/right
```

Double-click folders and supported archives to browse them. Double-click a file
to open its built-in viewer, or choose **Open externally** for other formats.
Use **F5** to copy marked files to the other pane and **F6** to move them.
**F5** extracts archive entries into a new folder. **Operations → Archive tools**
edits local ZIP/TAR/7z containers or creates/unlocks encrypted ZIP and 7z archives. Nested and
remote archives open read-only. **Mod+Z / Mod+Shift+Z** undo/redo eligible local operations and
**Mod+J** opens the queue and history. **Mod+P** toggles preview. Additional tools are available in the **File**, **Operations**,
**Navigate**, and **Selection** menus.

## Desktop appearance

The interface uses Slint's **Cupertino** style on macOS, **Fluent** on Windows,
and **Cosmic** on Linux. macOS keeps its native traffic-light window controls
and AppKit menu bar. Windows and Linux keep their desktop window decorations.
Slint discovers installed system fonts at runtime. The compact icon toolbar
shares the macOS title bar, keeping the native window controls. Breadcrumbs are
the normal location display; **Mod+L** or the pencil button opens the full path
for editing. Toolbar actions have hover hints and accessible labels.

Open **Settings → Desktop appearance** to choose the colors and layout profile.
**Automatic** detects the platform, including Ubuntu's orange accent.
**Color scheme → Follow system** follows the desktop's light/dark preference;
Light and Dark overrides are available.

Slint renders the controls in the selected platform style. Widget styles are
chosen at build time; the runtime appearance profiles control colors, font size
and layout. Cosmic does not reproduce every Ubuntu/GNOME theme. See the
[Slint guide](docs/SLINT_UI.md#platform-styles) for style overrides and desktop
integration details. Windows/Linux native interactions still need validation
on those desktops.

New macOS installations hide the function-key command bar by default. Enable
it in Settings if desired; existing preferences are preserved.

## Keyboard shortcuts

**Mod** means **Cmd** on macOS and **Ctrl** on Windows/Linux. Function keys may
require **Fn**. Common operation bindings can be changed in Settings; some
navigation shortcuts are fixed.

| Shortcut | Action |
| --- | --- |
| Tab | Switch pane |
| Arrows / Page Up / Page Down / Home / End | Navigate |
| Enter / Backspace | Open / parent folder |
| Space / Insert | Mark or unmark |
| Shift + arrows or click | Extend selection |
| Mod + click | Toggle an item |
| F1 / F2 / F3 / F4 | Help / rename / view / edit |
| F5 / F6 / F7 / F8 | Copy / move / new folder / Trash |
| Mod + L / F | Focus location / filter |
| Mod + Shift + F | Recursive search |
| Mod + T / W | New tab / close tab |
| Mod + A / D / R | Mark all / bookmark / refresh |
| Cmd + Shift + . (macOS) / Ctrl + H (Windows/Linux) | Show hidden files |
| Mod + , | Settings |
| Cmd + Delete (macOS) | Move to Trash |
| Mod + C / X / V | System file copy / cut / paste |
| Mod + Z / Mod + Shift + Z | Undo / Redo |
| Mod + J | Persistent queue and history |
| Mod + P / Mod + Shift + P | Preview pane / macOS Quick Look |
| Alt + drag | Native drag-out; GTK drag window on Linux |
| Mod + S | Save text |
| Alt + Left / Right | Back / forward |
| Escape | Clear marks/filter or close dialog |

## Remote connections

Open **File → Connections** to configure SMB2/3, SFTP, FTP, WebDAV, S3, HTTP, Dropbox,
Google Drive, OneDrive, GCS, or WebHDFS. SFTP currently requires macOS or Linux;
Windows offers the other adapters. Secrets can be kept in memory or stored
in the operating system's credential store. Google Drive, Dropbox and OneDrive support browser OAuth
login with PKCE and automatic token refresh, using your own registered client.
See [OAuth setup](docs/OAUTH.md); live account validation is still pending.

Remote testing covers a filesystem adapter and a loopback WebDAV server. Live
remote services have not been validated. Read the
[connection guide](docs/REMOTE_CONNECTIONS.md) for authentication requirements,
service-specific behavior, and transfer limits.

## Configuration

Tabs, named workspaces, bookmarks, connection profiles, and preferences are saved in
`settings.json` in the operating system's application configuration folder.
Invalid settings are reported and preserved. Undo snapshots and operation
records are stored in a sibling `history/` directory; full recovery copies can
grow substantially. Retention targets and optional automatic cleanup are available.
Queue recipes are saved separately; passwords and editor drafts are excluded. See the
[recovery and queue guide](docs/FEATURES_0.5.md#recovery-and-queue-details).

Use `RUST_COMMANDER_CONFIG_DIR` for an isolated configuration. On macOS/Linux:

```sh
RUST_COMMANDER_CONFIG_DIR="$PWD/.runtime/config" ./scripts/run.sh
```

On Windows, set `$env:RUST_COMMANDER_CONFIG_DIR` before running Cargo.

Settings from the earlier muCommander Rust preview are read when the new
configuration file does not yet exist, then saved under Rust Commander's
configuration directory. The original settings are preserved. The old
`MUCOMMANDER_CONFIG_DIR` variable remains a fallback; `RUST_COMMANDER_CONFIG_DIR`
takes precedence. Previously saved credentials remain accessible through their
legacy credential-store entries; new credentials use the `rust-commander` service.

## Development

On macOS/Linux, launch a debug build with `./scripts/run.sh`. Run the checks
used by CI with:

```sh
./scripts/cargo-local.sh fmt --all -- --check
./scripts/cargo-local.sh clippy --locked --all-targets -- -D warnings
./scripts/cargo-local.sh test --locked --all-targets
./scripts/cargo-local.sh build --locked --release
```

On Windows, replace `./scripts/cargo-local.sh` with `cargo`.

The Slint UI is compiled from `ui/commander.slint` by `build.rs`. SVG assets are
embedded in the executable. See the [Slint guide](docs/SLINT_UI.md) for platform
styles, source layout, input integration and reproducible screenshots.

The [CI workflow](.github/workflows/ci.yml) runs on pushes, pull requests, and
manual dispatch. It checks formatting, then runs Clippy, tests, and release
builds on Ubuntu, macOS, and Windows. Successful platform jobs upload build
artifacts to the workflow run's summary page for **7 days**, including an ad hoc
signed macOS app bundle. These are CI builds, not release installers; native
runtime dependencies still apply. The badge above shows the `main` branch.
Local validation is recorded in [parity/VALIDATION.md](parity/VALIDATION.md).

The macOS/Windows drag dependency is a small vendored subset of `drag` 2.1.1.
Its [provenance and update instructions](vendor/drag/README.md) explain why the
unused GTK3 backend is excluded. Linux uses the separate GTK4 helper.

Tests use temporary directories and generated archive/PDF fixtures. The WebDAV
integration test opens a loopback-only server; it requires permission to bind
a local port but no live service credentials or user files.

Build artifacts in `target/`, the optional toolchain/cache in `.tools/`, and
local test state in `.runtime/` are ignored by Git. Build output can grow to
several gigabytes; `cargo clean` removes it, including packaged apps under
`target/`, and the next build recompiles dependencies.

## Roadmap

The [upstream parity checklist](parity/PARITY.md) tracks 166 actions and 34
protocol, archive, and viewer modules against a pinned source revision.
Outstanding work includes:

- Validation of Windows, Linux, live remote services, and native drag-and-drop.
- NFS/ADB and virtualization providers, Bonjour discovery, and Windows registry
  browsing. Direct SMB2/3 access is available; broader NAS/server coverage remains open.
- Live validation of browser OAuth, keychain prompts, and remote rename/conflicts.
- Remote archive editing and broader archive format/metadata fixtures.
- Remote/resumable multi-file transfers, remote synchronization, and broader
  Undo/Redo coverage beyond the journaled local operations.
- Additional languages, remaining detailed diagnostics, upstream XML settings
  migration, and broader text-encoding, viewer, and terminal compatibility.

To refresh the upstream inventory while preserving recorded statuses:

```sh
python3 scripts/audit-upstream.py /path/to/upstream
```

Check the completion gate with `python3 scripts/check-parity.py`. It currently
returns a nonzero exit status because full parity has not been achieved.

## Contributing

Bug reports, documentation fixes, tests, and pull requests are welcome. Before
starting a larger change, open an
[issue](https://github.com/ghillert/rust-commander/issues) to discuss its scope.
Check the parity checklist for existing gaps and include the relevant upstream
behavior when proposing compatibility work.

For bug reports, include the operating system, app version, reproduction steps,
and expected versus observed behavior. Use disposable sample files where
possible, and remove credentials from logs or screenshots. For code changes,
run formatting, Clippy, and the relevant tests, and describe the validation
performed in your pull request.

## License

Distributed under **GPL-3.0-only**. See [LICENSE](LICENSE) for the terms and
[NOTICE](NOTICE) for upstream attribution, third-party licenses and test-fixture
notices. Dependencies retain their own licenses.

This project is an independent Rust implementation inspired by muCommander;
it is not an official muCommander release.
