# Icons in Rust Commander

## Choosing file icons

In **Settings → Desktop appearance → File icons**, choose:

- **System with built-in fallback** (default): preserve native application,
  custom, and document icons. A confirmed generic OS document icon yields to
  a specific built-in type icon. This includes stock macOS document templates
  with a faint type label, such as Preview's PDF and image icons.
  If no specific type is recognized, the generic
  native icon remains. Failed native lookups use built-in icons.
- **Built-in icons only**: use the bundled catalog without querying native icon
  services. This works consistently across machines and desktop themes.

Changes take effect in every open window when Settings is accepted, persist
across restarts, and are saved with workspaces. Cancelling Settings preserves
the previous choice. The stored preference is `file_icon_source`, with values
`system_with_fallback` and `built_in`; older settings use the first option.

Built-in icons appear immediately while native icons load in the background.
Archive and remote entries always use the built-in catalog without probing local
paths. Matching uses filenames only, so no content is read and no remote files
are downloaded to determine an icon. Symbolic links receive a link badge when
using built-in icons and the listing supplies link metadata.

Native APIs do not consistently report whether an icon is generic. macOS checks
custom-icon resource metadata and the FinderInfo custom-icon flag, then compares
against native generic-document references. It also recognizes stock document
templates when the default handler is a system application, its matching document
declarations provide no icon artwork, and the file's normalized icon exactly
matches the type's default artwork. This applies across file types rather than
special-casing PDFs. Third-party handlers, declared document artwork, per-file
custom icons, and uncertain metadata remain native; white or monochrome artwork
alone never triggers replacement. Windows compares system-image indices
against an unregistered extension. Linux compares resolved icon files with generic
theme references, preserving explicit file icons. Uncertain native results are
retained; a monochrome document icon can still be a valid native type icon.

The queue and per-window native cache are bounded. Only the viewport and a small
scroll margin request icons, off-screen rows release native images, cancelled
requests are not cached as missing icons, and late results cannot override
built-in-only mode. Cached native results expire after a minute, allowing changed
associations and desktop themes to be picked up.

## Artwork and coverage

Application controls use Fluent regular icons. The sidebar uses Fluent Color
where available, and accent-tinted Fluent regular icons for concepts without a
color variant, including downloads, computers, drives, and connections. Sidebar
icons are 20 logical pixels; file-list icons remain 18 logical pixels. Native
window controls, toolkit widget decorations, and Rust Commander's logo retain
their existing artwork.

The curated Material Icon Theme catalog covers PDF, office documents, text and
Markdown, XML/JSON/YAML/TOML/configuration, spreadsheets and databases, archives
and disk images, images/audio/video/fonts, EPUB, executables and scripts, common
programming languages, and Git/Docker/build/configuration filenames. Exact
filenames take precedence over explicit filename families such as `.env.*`,
then the longest matching compound extension is used. Matching is ASCII
case-insensitive; unknown files and folders use generic built-in artwork.

Multicolor icons are never tinted. Monochrome controls use the current UI colors;
disabled multicolor controls retain their hues at reduced opacity. Applicable
light-theme variants switch with the application's appearance. File icons are
not content thumbnails; image/PDF previews remain separate features.

## Updating assets

`ui/icons/catalog.json` is the source of truth for semantic IDs, rendering
treatment, filename rules, immutable upstream revisions, source paths, checksums,
and transformations. Only selected SVG assets are bundled; the complete VS Code
extensions, JavaScript packages, icon fonts, and downloadable themes are not
required. Ordinary builds and catalog checks work offline.

After editing mappings or adding assets from the pinned revisions:

```sh
python3 scripts/icons.py --fetch
python3 scripts/tests/test_icons.py
```

To deliberately update an upstream library, edit its full commit SHA in the
catalog and run `python3 scripts/icons.py --fetch --refresh`. Review the asset
and license changes, then run the tests above. `--refresh` deliberately replaces
source/output checksums. An interrupted fetch can be retried with `--fetch`;
already verified assets are reused and missing/corrupt assets are fetched again.

The importer generates the Slint asset mapping, Rust filename tables, and the
marked icon-license section of `NOTICE`. It validates SVGs, pinned checksums,
references, and unused assets. The generic Material folder derives from the
upstream folder template by removing its optional motive, a recorded deterministic
transformation. Generic file artwork uses Material's document icon. Other SVGs
retain their upstream bytes. `.gitattributes` preserves vendored bytes across
Windows and Unix checkouts. Full upstream MIT texts are retained in
`ui/icons/licenses/` and `NOTICE`, which is included in release packages.

## Validation

```sh
python3 scripts/tests/test_icons.py
./scripts/cargo-local.sh test --locked --all-targets --all-features
./scripts/cargo-local.sh clippy --locked --all-targets --all-features -- -D warnings
./scripts/cargo-local.sh build --locked --features screenshot
python3 scripts/icon-smoke.py
```

The last command uses disposable settings and renders every catalog icon at
16/18/20/24 logical pixels, plus selected and disabled examples, in light/dark
mode with software and FemtoVG. Galleries run at 1x, 1.5x, 2x and 3x; file lists
exercise both source modes. Output is under `target/icon-smoke/`. `--quick`
uses 1x only. Linux runs the quick matrix under Xvfb in CI. Native macOS/Windows
provider tests run with the normal suite; Linux's display-dependent provider
check runs separately under Xvfb. A passing API check on a Mac does not establish
Windows/Linux desktop behavior.

For this migration, the macOS all-targets/all-features suite, warnings-denied
Clippy, formatting, seven offline catalog/import tests, and release-automation
tests passed. All 24 visual renders completed on this Mac; the light file list,
selected dark file list, and full galleries were inspected, including FemtoVG
at 3x. Settings also rendered with isolated English, German, and Spanish
configurations; the translated icon selectors were visually checked.
The macOS provider's explicit-custom-icon check passed, and the packaged
app's strict ad hoc signature and complete bundled icon notices were verified.
Windows cross-target compilation/Clippy and a Linux GTK API harness passed;
Windows/Linux native desktop execution was not available locally. The CI checks
above are configured for the next workflow run.

The subsequent macOS template-fallback correction was checked with PDF, JPEG,
PNG, text, RTF, and ZIP files. Native tests cover stock templates, explicit custom
icons (even when assigned the stock artwork), document icon declarations,
malformed metadata, and artwork mismatches. Clippy and formatting passed, and
the 12-render quick matrix was repeated with the default system/fallback mode
and built-in-only mode. Light and selected-dark file lists were inspected with
the corrected fallbacks and retained application icons.

Measure release binary size using the same target, locked dependencies, feature
flags, environment, and release profile before and after importing new assets.
The size comparison for this migration is recorded in `docs/RELEASE_SIZE.md`.
