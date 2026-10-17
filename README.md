# muCommander Rust

[![CI](https://github.com/ghillert/rust-commander/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ghillert/rust-commander/actions/workflows/ci.yml)
[![Rust 1.92+](https://img.shields.io/badge/Rust-1.92%2B-orange?logo=rust)](Cargo.toml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)

A dual-pane desktop file manager written in Rust, inspired by
[muCommander](https://github.com/mucommander/mucommander). Browse local files and
archives, manage transfers, and work in an embedded terminal from one window.
The interface uses egui/eframe and runs without a JVM or browser engine.

> **Development preview — 0.3.0.** This is an independent implementation with
> full upstream compatibility still in progress. Apple Silicon macOS has been
> validated; Windows and Linux builds and desktop integration still need testing.
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
  compressed streams; read additional formats through libarchive.
- **File tools:** batch rename with preview, recursive name/content search,
  folder comparison, checksums, properties, permissions, and split/combine.
- **Built-in viewers:** UTF-8 text editing, images, hex, and PDF page navigation.
- **Embedded terminal:** a PTY-backed shell in the active local folder; hiding
  the terminal preserves its session.
- **Desktop appearance:** macOS, Windows, Ubuntu, and GNOME profiles, installed
  system fonts, light/dark mode, and native macOS menus and window controls.
- **Remote adapters:** saved connection profiles and streaming transfers for
  SFTP, FTP, WebDAV, S3, and several cloud services. Validation is limited; see
  [Remote connections](#remote-connections).

See [file operations and format support](docs/FILE_OPERATIONS.md) for supported
formats, recovery behavior, and current limits. Local pane and external-file
drop handling is implemented and tested with egui events; native desktop
drag-and-drop integration remains unverified.

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
  build-essential pkg-config libarchive-dev libdbus-1-dev \
  libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxinerama-dev \
  libgl1-mesa-dev libxkbcommon-dev libwayland-dev
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
open "target/release/muCommander Rust.app"
```

The bundle uses an ad hoc signature. Notarization and release installers are
not configured.

## Usage

The executable accepts up to two starting directories:

```text
mucommander-rust [LEFT_DIRECTORY] [RIGHT_DIRECTORY]
```

Pass them after `--` when launching through Cargo. For example, on macOS/Linux:

```sh
./scripts/cargo-local.sh run --locked --release -- /path/to/left /path/to/right
```

Double-click folders and supported archives to browse them. Double-click a file
to open its built-in viewer, or choose **Open externally** for other formats.
Use **F5** to copy marked files to the other pane and **F6** to move them.
Archive contents are read-only; **F5** extracts selected entries into a new
folder. Additional tools are available in the **File**, **Operations**,
**Navigate**, and **Selection** menus.

## Desktop appearance

Open **Settings → Desktop style** to choose a profile. **Automatic** selects
macOS on a Mac, Windows on Windows, Ubuntu on Ubuntu, and a GNOME-style fallback
on other Linux systems. **Appearance → Follow system** follows the desktop's
light/dark preference; Light and Dark overrides are available.

The macOS profile includes the installed San Francisco font, a tinted Favorites
sidebar, blue folders, striped file lists, and a compact toolbar integrated into
the native title bar. AppKit provides the menu bar and traffic-light window
controls. Windows and Linux keep their system window decorations and use
installed Segoe UI, Ubuntu, or GNOME fonts where available, with a bundled
fallback. System fonts are loaded at runtime and are not redistributed.

The file lists and controls are drawn by egui. Profiles approximate desktop
conventions; they do not use native AppKit, WinUI, or GTK widgets throughout,
and the Linux fallback does not reproduce every desktop theme. Windows and
Linux appearance still require testing on those systems.

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
| Mod + C / X / V | In-app file copy / cut / paste |
| Mod + S | Save text |
| Alt + Left / Right | Back / forward |
| Escape | Clear marks/filter or close dialog |

## Remote connections

Open **File → Connections** to configure SFTP, FTP, WebDAV, S3, HTTP, Dropbox,
Google Drive, OneDrive, GCS, or WebHDFS. Secrets can be kept in memory or stored
in the operating system's credential store. Cloud connections currently use
supplied tokens; browser OAuth login and refresh are not implemented.

Remote testing covers a filesystem adapter and a loopback WebDAV server. Live
remote services have not been validated. Read the
[connection guide](docs/REMOTE_CONNECTIONS.md) for authentication requirements,
service-specific behavior, and transfer limits.

## Configuration

Tabs, bookmarks, connection profiles, and preferences are saved in
`settings.json` in the operating system's application configuration folder.
Invalid settings are reported and preserved.

Use `MUCOMMANDER_CONFIG_DIR` for an isolated configuration. On macOS/Linux:

```sh
MUCOMMANDER_CONFIG_DIR="$PWD/.runtime/config" ./scripts/run.sh
```

On Windows, set `$env:MUCOMMANDER_CONFIG_DIR` before running Cargo.

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

The [CI workflow](.github/workflows/ci.yml) runs on pushes, pull requests, and
manual dispatch. It checks formatting, then runs Clippy, tests, and release
builds on Ubuntu, macOS, and Windows. Successful platform jobs upload build
artifacts to the workflow run's summary page for **7 days**, including an ad hoc
signed macOS app bundle. These are CI builds, not release installers; native
runtime dependencies still apply. The badge above shows the `main` branch.
Local validation is recorded in [parity/VALIDATION.md](parity/VALIDATION.md).

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
- SMB/NFS/ADB and virtualization providers, Bonjour discovery, and Windows
  registry browsing. Mounted SMB/NFS volumes are already usable as local folders.
- Browser OAuth login/refresh, remote rename, and remote archive workflows.
- Archive editing, encryption, and broader format coverage.
- Native OS file clipboard, drag-out, undo/redo, and richer conflict prompts.
- Localization, upstream XML settings migration, configurable columns/toolbar,
  and broader text-encoding, viewer, and terminal compatibility.

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
[NOTICE](NOTICE) for upstream attribution and test-fixture notices. Third-party
dependencies retain their own licenses.

This project is an independent Rust implementation inspired by muCommander;
it is not an official muCommander release.
