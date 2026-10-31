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

`ui/chrome.slint` contains the shared chrome components and colors. Dialogs keep
form rows at their natural height, align content at the top and reserve space
around focus rings. Short confirmations use compact widths and content-based
height estimates; large forms keep a scrolling body and fixed action footer.

`ui/dialog-controls.slint` supplies rounded macOS text fields and push buttons
when the compiled style is Cupertino. Fields retain the shared text editor's
selection, clipboard, Undo/Redo and context menu; password values are masked and
excluded from their accessibility value. Buttons have disabled, focus and press
states. Non-destructive confirmation actions use the accent color; destructive
actions do not receive default emphasis. The other widgets, including pop-up
choices, retain Cupertino styling at their normal control height. Fluent and
Cosmic builds keep their standard Slint fields and buttons and use their larger
control metrics. Windows places dismissal after the action buttons; macOS and
GNOME-style layouts place it before them. Footer spacing is 8 logical pixels.
The main chrome also uses the compiled family's neutral surfaces and corner
radii instead of applying the same macOS-like tint everywhere. A runtime
color-profile change does not switch widget skins.

These are Slint-rendered controls, not embedded AppKit/WinUI/GTK controls. The
layout follows Apple's guidance on [text fields](https://developer.apple.com/design/human-interface-guidelines/text-fields),
[buttons](https://developer.apple.com/design/human-interface-guidelines/buttons)
and [sheets](https://developer.apple.com/design/human-interface-guidelines/sheets).
The monochrome SVG glyphs are original project assets.

See the [desktop UX review](DESKTOP_UX_REVIEW.md) for the Windows/Linux review,
source guidelines and remaining native acceptance work.

## Workflows

The Slint frontend includes both file panes, tabs, breadcrumbs, editable paths,
filtering, sorting, multiple selection, configurable columns and toolbar,
favorites, devices, connections, the configurable command bar and progress display.
The active sort column shows an up chevron for ascending order or a down chevron
for descending order, independently for each pane and tab. Parent-directory rows
show an up-arrow icon followed by `..`. Clicking anywhere on this row highlights
it; a second click, double-click or Enter opens the parent directory. File marks
are preserved when highlighting the parent row.
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

The [drag-and-drop guide](DRAG_AND_DROP.md) covers targets, copy/move confirmation,
conflict rules, hover navigation, scrolling, deferred virtual exports and native
interoperation. Slint retains in-window gestures; leaving the window hands a
local file list or deferred virtual offer to AppKit, OLE, XDND or Wayland. Native calls run after the
controller borrow ends. A release-time target refresh handles fast internal gestures; window-exit events and native pointer polling hand external gestures off while their button is held.

Incoming native file events are grouped and routed using frozen drop coordinates,
with dedicated folder/tab/tree/sidebar targets. If a desktop cannot provide
coordinates, the active-pane fallback opens an explicit destination confirmation.
Native offers acknowledge Copy while confirmation is pending; source deletion
requires a confirmed successful Move through the normal transfer engine.

Windows in the same process exchange virtual selections with expiring, single-use
private offers. The receiving window uses the original typed location and opens
guarded confirmation without preparing an export. Native receivers handle AppKit
file URLs/private promises, OLE private metadata/CF_HDROP, and X11/Wayland private
handoffs. Windows OLE supports asynchronous extraction and a message-pumping
fallback for synchronous receivers. Physical desktop acceptance remains separate
from the protocol/provider fixtures recorded in the parity tracker.

Folder comparison uses local/remote typed pane locations and streaming SHA-256.
Synchronization retains those locations through preview, invalidates edited
previews, and filters choices by destination write capability.

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

To check a real transfer progress bar with a throttled loopback HTTP server:

```sh
./scripts/cargo-local.sh build --locked --features screenshot
python3 scripts/progress-smoke.py
```

On Linux, run the Python command under `xvfb-run -a dbus-run-session --`.
The fixture downloads into disposable folders and verifies a nonzero percentage
and speed with both software/light and FemtoVG/dark rendering. Images and logs
are written under `target/progress-smoke/`. Linux CI runs the same check.
Screenshot builds also accept `RUST_COMMANDER_SCREENSHOT_SEQUENCE`, a JSON array
of `[delay_ms, callback, value]` steps; supported callbacks are `navigate`,
`open`, `click`, `menu`, `action`, `dialog`, `select`, `field`, `sort`, `parent` and `viewer`.
Sort values contain an encoded JSON `[pane, column]` pair with zero-based indices.
Parent values contain an encoded JSON `[pane, open]` pair; `false` clicks the row
and `true` opens it directly.
Open/click values are native file paths. Field values contain an encoded
JSON `[field_id, value]` pair. This drives the existing UI callbacks for reproducible
multi-step checks; it is compiled out of normal development and release builds.
On macOS, `native-field`, `native-field-return`, `native-action` and `native-check` exercise the real
Extract sheet. Use `scripts/native-extract-smoke.py` for its disposable fixtures;
the script freezes its executable before running, so a separate development
build cannot replace the binary between scenarios. Run desktop keyboard
inspection separately from GUI smoke tests to keep their focus independent.

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

### Native Extract sheet on macOS

Extract uses AppKit text fields, pop-up buttons, checkboxes and push buttons in a
sheet attached to its file window. Its fields and actions use the same dialog
model, controller, conflict scan and operation guards as the Slint fallback.
Other forms continue to use Slint. See [native forms](NATIVE_FORMS.md) for the
adapter boundary, cross-platform integration findings and acceptance limits.

### Bottom command buttons

The command bar is visible by default on every platform, including macOS.
Existing macOS preferences from before this default change are migrated once to
show the bar. After migration, an explicit choice to hide it in Settings is
preserved. Native dialog styling does not remove the Commander action buttons.
Unit tests use a private process-wide configuration directory, shared with their
operation workers, so adapter autosaves cannot overwrite desktop preferences and
worker recovery data remains available for Undo.

### macOS dropdown indicators

Slint-rendered form and viewer dropdowns use a neutral face with unboxed up/down
chevrons on macOS. `DialogChoice` keeps the standard ComboBox as its interactive
and accessible control, including its popup, focus ring, keyboard navigation and
selection callback. The macOS face has no input handlers, so clicks over the
indicator still reach the underlying control. Fluent/Cosmic visuals are retained;
the actual AppKit Extract sheet continues to use its system NSPopUpButton.
