# Native operation forms alongside Slint

[← Slint interface](SLINT_UI.md)

Reviewed on 2026-10-30. Slint remains the main file-manager interface. The first
native form is macOS **Extract archive**, implemented as an asynchronous AppKit
sheet attached to the originating file window. Windows and Linux retain their
working Slint Extract forms while their native integrations are evaluated.

## Shared behavior and macOS implementation

`dialogs::Dialog` and `Field` remain the canonical form description. The AppKit
renderer consumes their existing IDs, raw choice values, translated labels,
busy state, error and button state. Native callbacks enqueue field edits and
commands; the ordinary controller applies them and starts the same background
conflict scan and extraction operation. Native controls never perform filesystem
work. There is no second set of extraction validation or overwrite rules.

The sheet includes a destination field, global conflict policy, merge checkbox,
per-file conflict choices, recoverable-replacement explanation, inline errors,
Apply to all, Cancel and the default Extract action. Long conflict lists scroll.
Destination edits retain their cursor/selection when conflict results change the
form. Return from the destination submits; an early Return waits for pending
preflight and is cancelled if the operation inputs change, an error occurs or
the window exits. Escape and the sheet close control cancel. Changing focus does not submit. Cancellation never starts extraction.
The native Edit menu routes Select All, Cut, Copy, Paste, Undo and Redo through
AppKit's responder chain, including while the file commands are blocked by a
modal sheet. Text Undo therefore stays inside the native editor.
AppKit supplies the system controls, text editor, focus ring, accent and appearance;
explicit light/dark app preferences are respected. Native accessibility labels
identify the destination and conflict choices.

The presentation request is collected during adapter synchronization and applied
**from a posted event-loop event, after both the adapter and shared runtime
borrows are released**. AppKit can
synchronously send winit focus notifications while attaching or ending a sheet.
Keeping Cocoa presentation outside these borrows prevents reentrant controller
borrows. Posting the work also keeps sheet animations outside Slint's timer
callbacks: Cocoa may pump its own event loop during presentation or dismissal.
The dispatcher coalesces pending updates and defers recursive presentation. Native menu enablement follows the key sheet as well as the parent
window. Sheet delegates/targets remain retained until dismissal and weak delegates
are disconnected during teardown. A missing AppKit parent uses the Slint fallback,
including in the headless testing backend.

Only target-specific AppKit feature flags were added. The implementation reuses
the project's existing objc2/AppKit packages and the system AppKit framework.

## Windows integration finding

The official Rust `windows-reactor` package is a **preview**, distinct from the
C# Microsoft.UI.Reactor project returned by the documentation index. Its current
source's `App::run_component` calls a private `run` function which initializes an
STA thread and enters WinUI `Application::Start`. This is an owning application
loop; it is not an API for inserting controls into an existing winit loop.
Calling it inside a Slint callback is therefore not the integration design.

A separate STA dispatcher or helper process needs a Windows prototype before this
backend can be enabled. That prototype must prove native owner-HWND association,
modal input blocking limited to the originating window, close/crash recovery,
keyboard routing, DPI scaling and application shutdown while Slint remains live.
A helper process is an isolation option, not a validated implementation.
The matching Windows App SDK runtime must either be installed or staged with
`windows-reactor-setup`; this changes deployment beyond the existing windows-sys
bindings. The preview dependency has not been added to production Cargo targets.
No Windows desktop or WinUI runtime test was available on this macOS host.

Sources: [Microsoft reactor readme](https://github.com/microsoft/windows-rs/blob/master/crates/libs/reactor/readme.md),
[application startup source](https://github.com/microsoft/windows-rs/blob/master/crates/libs/reactor/src/native/app/mod.rs).

## Linux integration finding

GTK4 0.10.3 is already a project dependency. The checked Linux development image
provides GTK **4.8.3**, Xvfb and Weston; **libadwaita is not installed**. Adding
libadwaita requires a compatible Rust binding version and native build/runtime
packages. The current Linux all-target/all-feature Clippy check passes with the
native macOS code excluded. That establishes fallback compilation, not GTK form
or compositor acceptance.

The existing icon worker initializes GTK on its own thread and processes GLib
notifications only while serving icon requests. GTK objects must remain on that
initializing thread. A form cannot be created on Slint's main thread after this
initialization, and the current blocking icon-worker loop cannot drive a live
form while idle. Supported options to prototype are a continuously dispatched
GTK service handling both icons and forms, or an isolated form helper (similar
to the existing legacy drag helper). Neither requires a nested GTK application
loop inside a Slint callback.

Parenting also needs real compositor checks. A winit window is not a GtkWindow,
so GtkWindow.set_transient_for cannot directly receive it. X11 needs a foreign
owner-window relationship; Wayland requires export/import of a supported foreign
parent handle. GDK's set_transient_for_exported accepts an exported handle, not a
raw wl_surface pointer. Missing compositor support needs a documented fallback.
Libadwaita follows GNOME conventions, rather than reproducing KDE Breeze.

Sources: [GTK threading](https://github.com/gtk-rs/gtk4-rs/blob/main/gtk4/README.md),
[libadwaita setup](https://gtk-rs.org/gtk4-rs/stable/latest/book/libadwaita.html),
[Wayland foreign parenting](https://docs.gtk.org/gdk4-wayland/method.WaylandToplevel.set_transient_for_exported.html).

## Checks and remaining acceptance

Run the actual AppKit smoke fixtures on macOS after a screenshot-enabled build:

```sh
scripts/cargo-local.sh build --locked --features screenshot
python3 scripts/native-extract-smoke.py
```

The fixture edits native controls and dispatches Cocoa target/action callbacks,
checking extraction output, cancellation, native close, global Skip and per-file conflict choices, Apply to all, immediate/delayed Return,
application quit,
inline preflight errors, long lists and reopening with disposable files. The
headless fixture checks the shared model and fallback cancellation. AppKit cached
view PNGs do not reproduce every composited background; desktop visual inspection
is separate from those snapshots. Actual VoiceOver speech, Full Keyboard Access
settings, mixed-DPI monitor moves, cross-window modal behavior and packaged native
Windows/Linux forms remain acceptance work. Native accessibility-tree inspection
alone is not VoiceOver acceptance.
