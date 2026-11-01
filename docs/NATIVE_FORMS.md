# Native controls alongside Slint

[← Slint interface](SLINT_UI.md)

Reviewed on 2026-10-31. Rust Commander uses actual platform controls for editable
forms, short confirmations and Settings. Slint still owns the file-manager
surface and the shared application controller.

| Surface | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Settings | AppKit `NSTabView` with system fields, popups and buttons | Win32/common-control tabs, edits, combos and buttons | GTK4 notebooks, entries, dropdowns and buttons |
| Copy, Move, Rename, Extract, Pack and other short forms | Asynchronous AppKit sheet | Owned Win32 form window | Modal GTK4 window with a foreign parent |
| File, folder and archive-output browsing | `NSOpenPanel` / `NSSavePanel` | Shell `IFileDialog` | `GtkFileChooserNative` (portal/system chooser where available) |
| Main menu | AppKit command menus | Shared Slint menu | Shared Slint menu |
| File icons, clipboard, window frames and drag/drop | Existing native integrations | Existing native integrations | Existing native integrations |
| Panes, toolbar, sidebar, context menus, viewers and terminal | Shared specialized Slint UI | Shared specialized Slint UI | Shared specialized Slint UI |

## Coverage and intentional shared surfaces

The native renderer accepts Settings; ordinary prompts, including permissions
and deletion; packing; Connections; Archive tools; Batch rename; synchronization
and recovery forms; Open as/Open with; exit confirmation; results; browser URLs;
donation; email; external-editor confirmation; associations; updates; diagnostics
reports; Select by pattern; tab naming; Empty Trash confirmation; settings import
and imported-command review. Availability is based on the actual form schema, not its title.
Unsupported field types, more than 512 fields, 128 result rows or five footer
buttons use the shared renderer. Streaming search, comparison/diff, queue/history,
bookmarks/workspaces, event consoles and Help remain shared workflows.

Settings retains General, Folders, Appearance, Shortcuts, Mail and Advanced,
including browsing/columns and theme/toolbar/command-bar subpages. Platform tabs
size their labels naturally. Apply saves and stays open; OK saves and closes;
Cancel discards changes since Apply. Inline errors disable submission and offer
Review invalid setting to reveal the affected page. Shortcut filtering retains
paired primary/alternate edits and selection across structural changes.

Native path browsing is offered for local transfer/extraction destinations,
archive outputs and supported key/application/archive file fields. Extract
chooses a **parent** folder and preserves the proposed new folder name. Remote
URI destinations remain editable and never receive an unrelated local chooser.
Opening a chooser clears a pending early-Return submission; cancelling it leaves
the current field intact. The picker never copies, extracts or publishes files.

Custom panes keep Commander selection marks, virtualized rows, cross-pane drag
behavior and keyboard bindings. Viewers and the terminal need document/grid
rendering. Their Slint controls and context menus are intentional; embedding
foreign widget windows into the winit drawing surface is not part of this pass.

## Shared behavior

`dialogs::Dialog`, `Field` and `Row` are the canonical form description. Renderers
consume the same IDs, raw choice values, translated captions, busy/error state
and enabled actions. Native callbacks enqueue edits/actions; the ordinary
controller validates and starts the existing operation. Credentials are never
passed on a helper command line or stored in a protocol file.

Return is a safe default action only where appropriate. Delete/trash, Empty Trash
and discard-work exit prompts have no default Return action. An early Return on a transfer/extraction form
waits for preflight and retains the immutable source, destination and conflict
options; editing any of them, opening a chooser, cancellation, an error or owner
exit clears that intent. Text entry, Tab/Shift+Tab, Escape, disabled actions,
password masking and text selection use the platform widgets. Native controls
do not change overwrite guards, queueing, recovery, progress or Undo.

## AppKit

AppKit presents sheets from a posted event-loop event, after controller/runtime
borrows are released. Cocoa can synchronously send focus notifications or pump
its run loop during presentation/dismissal. Targets and delegates stay retained
until teardown; weak delegates are disconnected before controls are released.
Text selection survives conditional fields and shortcut filtering. The native
Edit menu routes text editing through the responder chain while a form is open.
AppKit supplies focus rings, accent, accessibility roles and explicit/system
light and dark appearance. A missing native parent uses the Slint fallback.

Settings uses compact real `NSTabView` controls rather than Slint's stretched
Cupertino segments. Read-only long output uses a native scrolling text view.
Screenshot fixtures capture only the app's own composited sheet; view caching
cannot reproduce AppKit's layer-backed tab backgrounds correctly.

## Windows

A private helper process owns the Win32 UI/message loop. The existing executable
handles `--native-forms-helper` before normal startup, so no second app session or
user configuration is loaded. The form is owned by the originating HWND; Slint
blocks input to that file window while other app windows remain usable. The
owner is never disabled across processes, so a helper crash cannot strand it.

The application manifest enables common-controls v6 and Per-Monitor V2 DPI.
Controls use the Windows message font, system colors, keyboard dialog navigation,
vertical scrolling, minimum sizing and DPI-change reconstruction with text
selection retained. Path browsing uses the modern Shell file dialog on an STA.
These are actual **Win32/common controls**. No preview Reactor or
Windows App SDK deployment dependency is introduced. Classic common controls do
not implement the app's explicit dark preference; complete Windows dark-mode,
high-contrast and mixed-monitor acceptance remains open.

## Linux

An isolated helper initializes GTK4 on its main thread and continuously drives
GLib. This avoids sharing GTK objects with the existing icon worker or nesting a
GTK loop inside Slint. X11 uses `WM_TRANSIENT_FOR`; Wayland exports the originating
surface through xdg-foreign v2 and imports that handle into GDK. Backend symbols
are detected at runtime. If export/import is unavailable, GTK initialization
fails or the helper cannot present, the shared Slint form remains available.

GTK uses the installed GTK theme, native accessibility roles and system widgets;
it is not libadwaita or a Qt/KDE widget implementation. Explicit light/dark uses
GTK's application preference; System restores the helper's initial preference.
Portal chooser availability depends on the desktop. Compositor-specific parenting,
KDE appearance and physical desktop accessibility still require acceptance.

## Lifetime and fallback

Windows/Linux form updates and events travel through anonymous private stdio
pipes with a 2 MiB message bound. Updates coalesce; structural rebuilds preserve
selected tabs and editable text selection. A helper that fails or does not become
ready within four seconds falls back for that form, avoiding a respawn loop.
Changing/closing a form kills and waits for its helper. EOF or Close terminates
the helper even while a system file chooser is open. Main-loop shutdown also
reaps remaining helpers. The helper contains no file-operation implementation.

## Repeatable checks

Build with `scripts/cargo-local.sh build --locked --features screenshot`, then:

```sh
python3 scripts/native-extract-smoke.py
python3 scripts/native-operation-smoke.py
python3 scripts/settings-smoke.py --exercise
```

On Linux, run under Xvfb and a session bus; on Windows use `python` and the `.exe`
binary. CI runs these native-control fixtures on all three platforms. Extract
covers real output, Cancel/Close, conflicts, Skip/Keep both/Apply to all, immediate
and delayed Return, errors, long lists, reopening and quit. Operations cover
Copy/Move/Rename, ZIP/7z, comments, encryption flags, background work and conflicts.
Settings captures all nine pages and exercises native Apply/Cancel and invalid drafts.
`native-helper-smoke.py` checks Windows/X11 control events, Close/EOF and EOF with
a system chooser open. Screenshot steps wait for native readiness and execute in
order rather than racing posted sheet presentation. Shared-controller tests cover
draft retention,
validation, hidden pages, shortcut filtering and headless fallback. Wayland drag
protocol fixtures check the shared connection/export lifetime.

Local evidence and desktop limitations are recorded in
[validation](../parity/VALIDATION.md). SDK compilation and Wine are supplementary;
they do not establish real Windows screen-reader, Explorer or mixed-DPI acceptance.
Actual VoiceOver/NVDA/Orca speech, Full Keyboard Access, physical monitor changes,
compositor coverage and cross-window modal interaction remain desktop checks.
