# Slint desktop interface

Rust Commander 0.7 uses **Slint 1.18.1** for its desktop interface. There is no
egui/eframe runtime, JVM, or browser engine. The existing Rust file, archive,
transfer, recovery, remote-service and terminal implementations remain in use.

## Platform styles

The build selects a standard Slint widget style for the target platform:

| Target | Default style | Window integration |
| --- | --- | --- |
| macOS | Cupertino | Native window controls and AppKit menus |
| Windows | Fluent | Native window frame and caption buttons |
| Linux | Cosmic | Window decorations provided by winit and the desktop |

These are Slint-rendered widget styles. Cupertino follows macOS conventions;
Cosmic does not reproduce every installed Ubuntu/GNOME theme. The Automatic
appearance profile detects Ubuntu and selects its orange accent. The existing
appearance profiles and light/dark preferences remain available in Settings.
System fonts are discovered by Slint on the running machine.

The widget style is chosen at **build time**. Override it when compiling:

```sh
SLINT_STYLE=fluent ./scripts/cargo-local.sh run --locked
SLINT_STYLE=cosmic ./scripts/cargo-local.sh run --locked
SLINT_STYLE=cupertino ./scripts/cargo-local.sh run --locked
```

On Windows PowerShell, set `$env:SLINT_STYLE` and use `cargo` directly. Removing
the environment variable restores the platform default on the next build.
The Qt style requires a Qt backend and libraries; this application uses winit.

## Desktop chrome

The main window uses compact icon actions, text breadcrumbs, quiet tabs,
sidebar icons, subtle row stripes and a slim function-key bar. The active pane
uses a small accent marker; the selected file remains the main visual focus.
Numeric size columns align to the right. Light and dark modes use the same
spacing and hierarchy.

By default, file rows use system-provided icons for local paths: AppKit `NSWorkspace` on
macOS, the Windows Shell, and GIO plus the GTK4 icon theme on Linux. A dedicated
worker handles native lookups and converts images into transparent RGBA pixels;
Slint images stay on the UI thread. Icons retain their colors when selected.
The 64-pixel images support high-density displays without tinting the artwork.

Only visible rows and a small scroll margin request icons. The request queue and
per-window cache are bounded; obsolete requests are cancelled and off-screen
model rows release their images. Cache entries expire after a minute to pick up
association or theme changes, and changed file timestamps have separate cache
keys. Remote and archive entries use embedded icons without requesting local
filesystem metadata. **Settings → Desktop appearance → File icons** also offers
built-in-only mode, which performs no native icon queries. The curated Material
catalog provides specific file-type fallbacks, including replacements for
confirmed generic native documents. See [the icon guide](ICONS.md) for matching,
custom-icon preservation, provenance, and visual checks.

Application icons use a shared generated catalog: Fluent regular controls,
Fluent Color sidebar icons where available, and accent-tinted regular variants
for the remaining sidebar roles. Multicolor icons bypass the alpha-mask tint
path. Sidebar icons are 20 logical pixels and file-list icons remain 18 pixels.

On macOS, the toolbar extends into the native title bar, with reserved space
for the close/minimize/full-screen controls. The native controls are vertically
centered in the header, including after resizing or changing toolbar visibility.
Dragging empty toolbar space moves the window. Hiding the toolbar keeps a
minimal title-bar area for the controls.
Other platforms retain their normal window caption above the toolbar.

The sidebar button beside Refresh hides or restores both Places and the
optional Folders tree, letting the file panes use the full width. The same
toggle is in **View → Toggle sidebar**, and **Settings → Show sidebar** keeps
the choice across launches. Places, Favorites, Devices and Connections can
still be collapsed independently by clicking their headings.

Use **Mod+L** or the pencil button to edit the full location. Enter navigates;
Escape returns to the file list. Text fields retain selection, clipboard,
Undo/Redo and context-menu editing. Toolbar controls have labels for assistive
technology, tooltips, keyboard focus and Enter/Space activation.

`ui/chrome.slint` contains the shared chrome components and colors. Standard
Slint widgets remain in forms and dialogs and follow the compiled platform
style. The monochrome SVG glyphs are original project assets.

## Workflows

The Slint frontend includes both file panes, tabs, breadcrumbs, editable paths,
filtering, sorting, multiple selection, configurable columns and toolbar,
favorites, devices, connections, the configurable command bar and progress display.
The [0.7 guide](FEATURES_0.7.md) covers quick find, advanced selection, locked tabs,
stacked panes, quick lists, encoding choices, Trash, multiple windows and XML import.

Its dialogs expose the existing file operations and conflict choices, archive
editing/encryption/unlocking, connections and OAuth, batch rename, recursive
search, synchronization, recovery storage, workspaces and transfer history.
Text/image/hex/PDF viewing, guarded text saves, the persistent preview and the
PTY terminal use Slint views. Settings retain the previous configuration format.

Operation dialogs keep the backend's validation and confirmation behavior.
Dropping files opens a confirmation or archive-edit form. It does not immediately
copy, overwrite or remove the source files. Closing dirty text asks before
discarding it. Single-pane mode shows the active pane and uses that same pane
for commands and drop targets.

## Desktop drag and drop

Slint's `DragArea`/`DropArea` handles drags between panes. The winit backend's
incoming OS file events are grouped into one confirmation for a multi-file drop.
Native AppKit, Win32 and X11 pointer queries identify the pane under the drop,
because winit does not report cursor movement during an external drag. If the
desktop prohibits that query, confirmation targets the active pane and displays
its destination for review. This bridge is needed by the pinned Slint
winit backend; it must be reviewed when upgrading Slint.

For outgoing OS drags, use **Alt + drag** on a local selection or
**Selection → Native drag-out**. macOS and Windows use the retained native drag
implementation with the Slint window handle. Linux opens the existing GTK4
helper window. Its file payload uses GTK4's native `GdkFileList` representation.
Native drag invocation is deferred until the controller is no longer borrowed.

## Source layout

- `ui/commander.slint`: widgets, layouts, focus scopes and input callbacks.
- `ui/chrome.slint`: toolbar controls, text fields, shortcuts, colors and glyph mapping.
- `ui/icons/`: pinned SVG artwork, licenses, source manifest and generated Slint catalog.
- `src/icons/`: generated filename tables; `scripts/icons.py` validates and updates both catalogs.
- `build.rs`: target style selection and resource embedding.
- `src/app.rs`: operation/controller state and shared validation/workers.
- `src/app/slint_ui/`: dialog models, commands, settings binding and keyboard input.
- `src/app/slint_ui/windows.rs`: independent window sessions and the shared queue/job/history runtime.
- `src/app/slint_ui/parity.rs`: quick find/lists, tab and layout commands, import and command-bar models.
- `src/navigation.rs`: selection, recent locations, persistent tab and layout state.
- `src/text_document.rs`: encoding/BOM/line-ending preservation and bounded text pages.
- `src/config_import.rs`: bounded XML parsing, supported migration mappings and backup commit.
- `src/ui_signal.rs`: worker notifications, clipboard commands and close requests.
- `src/viewer.rs`: renderer-independent raster data for image and PDF viewing.

The retained models update changed rows without rebuilding an unchanged
directory. Native multi-file drag payloads are shared between selected rows.
All windows use one desktop event loop and one transfer queue, active job and
recovery history. A job retains its originating window for completion handling;
closing another window does not cancel it. Preferences and saved collections
are coordinated across windows, while navigation, drafts and terminals remain
independent. The last saved window supplies the pane session restored on restart.

All UI updates run on the desktop event-loop thread; worker results arrive
through the existing channels. Dialog button models stay stable across worker
refreshes so a press/release sequence cannot lose its target. Closing a modal
restores file-pane focus. Slint normalizes macOS Command to its `control` key
modifier; terminal input separately maps physical Control to PTY control bytes.
A winit modifier-state bridge also handles synthesized OS shortcuts and clears
modifiers when the window loses focus. Terminal output is accessible by line,
and hiding it restores file-pane keyboard focus.
The macOS backend disables Slint's default menu to preserve the localized,
guarded AppKit command menus when the window regains focus.

## Validation and screenshots

Run formatting, warnings-denied Clippy and all tests as described in the README.
The Slint tests use its headless testing backend and temporary directories.
They cover real callback wiring, navigation, selection, editable paths,
confirmation guards, archive/remote drops, single-pane hit testing, shortcuts,
settings validation and dirty-editor close handling.

For a reproducible rendered client-area snapshot, use disposable folders and
an isolated configuration:

```sh
mkdir -p .runtime/slint-left .runtime/slint-right
RUST_COMMANDER_CONFIG_DIR="$PWD/.runtime/slint-config" \
RUST_COMMANDER_SCREENSHOT_TO="$PWD/.runtime/slint.png" \
  ./scripts/cargo-local.sh run --locked --features screenshot -- \
  "$PWD/.runtime/slint-left" "$PWD/.runtime/slint-right"
```

The app captures a frame after loading and exits. Set
`RUST_COMMANDER_SCREENSHOT_ACTION=Preferences` to capture Settings. Snapshots
can also open a command menu, for example with
`RUST_COMMANDER_SCREENSHOT_ACTION=Menu:Operations`, or collapse a sidebar
section with `RUST_COMMANDER_SCREENSHOT_ACTION=ToggleSection:places`.
Use `RUST_COMMANDER_SCREENSHOT_ACTION=ToggleSidebar` to capture the file panes
with the full navigation area hidden.
They cover the client area; native title-bar controls require a desktop smoke
check.
For machines without working OpenGL, `SLINT_BACKEND=winit-software` selects the
compiled software renderer. Linux desktop checks require a display or Xvfb.

See [the validation record](../parity/VALIDATION.md) for checks performed and
remaining platform/runtime limits. Headless tests and a cross-target compile
do not establish full Windows or Linux desktop behavior.

## Dependency updates

Keep `slint`, `slint-build`, the macOS winit backend and the testing backend on matching versions. The
project uses Slint's GPL-3.0-only licensing option. Its Rust 1.92 requirement
matches this project's minimum toolchain.

When updating, check all platform styles, native menus/window handles, external
file-drop routing, drag-out and the saved preference/shortcut behavior. The
[vendored native drag subset](../vendor/drag/README.md) remains necessary until
the corresponding native integrations can be replaced without regressions.

The window runtime uses [`run_event_loop_until_quit`](https://docs.slint.dev/latest/docs/rust/slint/fn.run_event_loop_until_quit)
and explicitly quits after the final protected session closes.

References: [Slint widget styles](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/),
[Slint Rust API](https://docs.slint.dev/latest/docs/rust/slint/).
