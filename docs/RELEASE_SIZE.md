# Release executable size

The release profile and dependency features are chosen to reduce executable size
while retaining the current desktop features. Size measurements refer to the
executable, not the build cache, application bundle, or compressed download.

## Measured comparison

Measured on Apple Silicon macOS with Rust/Cargo 1.99.1. The baseline was rebuilt
from commit `96c97d7` before these changes. All candidates use the same source,
locked package versions, target architecture, and symbol stripping. The two
candidates also include the dependency feature audit below.

| Build | Optimization / LTO / codegen units | Executable bytes | MB | Binary gzip bytes |
| --- | --- | ---: | ---: | ---: |
| Baseline | 3 / thin / 16 | 40,163,792 | 40.16 | 17,787,087 |
| Size candidate | s / fat / 1 | 24,734,192 | 24.73 | 12,489,378 |
| Selected profile | z / fat / 1 | 19,714,944 | 19.71 | 11,387,061 |

MB means 1,000,000 bytes. Gzip is measured on the executable alone, with level 6
and a fixed timestamp; it is not the size of the packaged application download.
The combined changes save **50.9%** of the uncompressed executable. The candidate
comparison does not isolate the two feature removals from the compiler settings.

The selected release profile uses `opt-level = "z"`, full LTO,
`codegen-units = 1`, symbol stripping, and `panic = "unwind"`. The `"z"` setting
disables LLVM's automatic loop vectorization; the explicit SIMD features used
by PDF/image dependencies remain enabled. This is a size/performance tradeoff,
not a claim of unchanged speed for every workload. Full LTO and a single codegen
unit also make clean release builds slower. Panic unwinding remains enabled so
worker failures can still be reported and recovered.

See [Cargo's profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html)
for the compiler options and their tradeoffs.

## PDF renderer contribution

A paired build experiment on the same Mac measures the incremental PDF cost:

| Isolated comparison | Executable bytes | MB |
| --- | ---: | ---: |
| PDF rendering enabled | 19,714,976 | 19.71 |
| PDF rendering removed | 17,010,672 | 17.01 |
| Incremental PDF cost | **2,704,304** | **2.70** |

This is **13.7%** of the optimized executable. The corresponding
executable-only gzip difference is **1,635,137 bytes (1.64 MB)**.

The experiment copies the source into a temporary directory and builds a separate
named binary with the same release profile and locked package versions. It then
replaces `viewer::render_pdf` with an error stub and removes Hayro in that copy.
The rest of the application, image formats and Slint UI stay in both builds.
This measures the net cost after link-time optimization, including PDF-specific
rendering, font handling, image codecs and embedded resources; shared code is
counted only where its removal actually reduces the executable.

The full control is 32 bytes larger than the normal executable. Both comparison
variants use exactly the same temporary source path and binary name. The normal
application keeps PDF support; the temporary no-PDF variant is not distributed.
The normal executable is verified byte-for-byte unchanged after the experiment,
and its optimized `pdf_viewer_renders_and_navigates_pages` test passes.

## Reproduce the comparison

Build the baseline in a separate checkout of `96c97d7`, then build the current
checkout with the same toolchain and native dependencies:

```sh
./scripts/cargo-local.sh build --locked --release
python3 -c 'from pathlib import Path; p = Path("target/release/rust-commander"); print(p.stat().st_size)'
```

On Windows, use `cargo` and `target/release/rust-commander.exe`. Do not compare a
compressed release asset with an unpacked executable or compare different target
architectures. To repeat the alternative profile experiment on the current
checkout:

```sh
./scripts/cargo-local.sh build --locked --release --config 'profile.release.opt-level="s"'
```

A subsequent plain `build --locked --release` restores the selected default.

## Dependency feature audit

The audit uses the locked dependency versions and all three desktop target
feature graphs. No dependency versions are upgraded.

| Dependency | Decision | Functionality retained |
| --- | --- | --- |
| OpenDAL | Disable defaults; keep Tokio execution and every previously selected service explicitly. Remove the unused `services-memory` backend. | Local filesystem adapter, FTP, WebDAV, S3, HTTP, Dropbox, Google Drive, OneDrive, GCS, WebHDFS, and Unix SFTP. The direct Reqwest dependency still enables Rustls TLS. |
| clipboard-rs | Disable its default `image` feature; keep `wayland`. The application calls its file, text, and custom-format APIs only. | File Copy/Cut/Paste, text clipboard, ownership markers, Windows drop effects, and Linux file formats. |
| Slint | Retain both FemtoVG and software renderers, accessibility, system text support, and X11/Wayland. | GPU and software rendering, accessible controls, international text, and desktop platform integration. |
| Hayro / image | Retain embedded PDF fonts, CMaps, SIMD, and all enabled image formats. | PDF rendering and BMP, GIF, ICO, JPEG, PNG, TIFF, and WebP previews. |
| Archives | Retain all existing compression and encryption features. | Existing ZIP, TAR, 7z, compressed stream, and libarchive support. |
| TLS crypto providers | Retain Ring and AWS-LC. OpenDAL's FTP dependencies enable both transitively; features are additive, so a direct default-feature override cannot remove the upstream request. | Existing TLS behavior and supported algorithms. Unifying these requires a separate upstream dependency change. |

The lockfile change removes only the unused clipboard-to-image dependency edge.
The preview image dependency is still enabled independently. Removing unused
features does not imply their full crate size was present in the final executable:
link-time dead-code elimination already discards unreachable code.

## Validation

Validation on Apple Silicon macOS:

- `cargo fmt --all -- --check` and warnings-denied all-targets/all-features Clippy pass.
- All **174 ordinary tests** pass in both debug and the selected release profile.
  These include PDF/image previews, archives and encryption, local/remote adapter
  contracts, loopback WebDAV, Undo/Redo, and reporting a panicked background worker.
  The live SMB and interactive clipboard cases remain excluded from ordinary runs.
- The interactive clipboard case is run separately and passes Copy/Cut/Paste,
  ownership and external-change checks. The original clipboard's PNG and TIFF
  data remain readable after restoration.
- The selected executable opens the desktop with both `winit-femtovg` and
  `winit-software`. PDF display and second-page navigation work with both;
  text and image viewing also work with the GPU renderer. These checks use
  disposable files and isolated settings. CLI help, version and invalid argument
  handling match the baseline.
- The normal macOS packaging script succeeds and strict code-signature validation
  passes. Every file-backed Mach-O code/data section matches the unbundled release
  executable after signing. Signing changes signature metadata and file length.
  The signed executable is **19,600,464 bytes**; the complete app
  bundle contains **19,641,469 bytes** of file contents.
- Windows clipboard APIs also pass warnings-denied Clippy with image support
  disabled in an isolated `x86_64-pc-windows-msvc` harness. The Windows/Linux feature
  graphs retain their existing services and UI features.

These measurements apply to macOS ARM64 only. Windows/Linux native release
builds, runtime validation, and sizes still require CI or separate machines.
Live remote services beyond the existing bounded tests have not been exercised.
No broad workload performance benchmark is claimed.


## Later feature build

After the next twelve feature additions, the Apple Silicon macOS optimized
executable measures **19,997,968 bytes** (2026-10-19). This is a separate feature
build; the before/after profile comparison above remains based on the same
original source revision. The [validation record](../parity/VALIDATION.md)
describes the tests and platform limits for the expanded application.


The subsequent stabilization build measures **19,998,032 bytes** on the same Mac.
It retains the profile above; the file-backed upload fix reuses bytes/memmap2
versions already in the lockfile. These later feature builds are separate from
the unchanged-source profile comparison.

## Configurable icon catalog

Measured on Apple Silicon macOS on 2026-10-20, using `aarch64-apple-darwin`
and the same toolchain, environment, and release profile for both builds:

| Build | Executable bytes | MB |
| --- | ---: | ---: |
| Native icons before this migration (`9c5b6fb`) | 20,181,632 | 20.18 |
| Configurable native/built-in icons and Fluent/Material catalog | 20,314,528 | 20.31 |
| Increase | **132,896** | **0.13** |

The increase is **0.66%**. Both binaries use `opt-level = "z"`, full LTO,
one codegen unit, symbol stripping, panic unwinding, and the default feature set
without the screenshot feature. These are unbundled executables, before package
relocation or ad hoc signing; download and `.app` sizes are different measures.

The baseline was rebuilt from an isolated `git archive` of `9c5b6fb`, followed by
the working tree build, using `scripts/cargo-local.sh build --locked --offline
--release` and the same target directory. The baseline source snapshot used
`--manifest-path target/icon-size/baseline-source/Cargo.toml --target-dir target`.
The migration adds the small `objc2-uniform-type-identifiers` framework binding
at the existing objc2 framework version for native generic-icon references;
existing dependencies were not upgraded. The catalog vendors 84 semantic icons
with 85 selected SVGs, including one light-theme variant.

The [icon guide](ICONS.md) describes the catalog, offline update/check procedure,
and validation matrix. Measurements on Windows/Linux still require native
release builds on those platforms.

The colorful-sidebar follow-up was measured against `224a02b` with the same
Apple Silicon target and release profile. Both executables are **20,314,528
bytes**; their contents differ, but the final file length is unchanged in this
build. The updated catalog contains 85 semantic icons and 86 selected SVGs,
including recorded palette adaptations for desktop and hard-drive artwork.
