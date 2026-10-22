# Native Wayland file drag and drop

Adapted from [limn-dnd 0.1.43](https://crates.io/crates/limn-dnd),
[upstream source](https://github.com/Niiihuel/limn) revision `b595149d9635df63fd898fa0f7f289b13d818634`.
Published crate SHA-256: `f9b6101d3268da4985d10a2357be2ae756a1ccd8350f6383077fd536c8aa9427`.

The adapter attaches a private event queue to the existing window's Wayland
connection. It supports incoming file lists with surface coordinates and outgoing
file drags without a GTK helper window. The host retains the Slint window handle
until the worker has shut down.

Local adaptations: preserve Rust 1.92 compatibility; expose initiating-button
state; bound URI-list size and pipe I/O deadlines; acknowledge
incoming offers as Copy while Rust Commander reviews and performs the requested
copy/move through its guarded transfer engine. This prevents a foreign source
from deleting files when a confirmation is cancelled. The application never deletes
sources solely in response to a native drag-finished event.

Local outgoing offers negotiate Copy/Move. Deferred virtual offers advertise Copy
and queue early URI-list pipes until `dnd_drop_performed`; cancellation closes the
queued pipes without calling the provider. The bounded pipe workers stay off the
Wayland event queue. Native completion alone never removes source files.

Same-process virtual selections advertise a per-offer private MIME type. Only
active locally known tokens are negotiated by this receiver. Private drops send
the token to the host without asking for file contents. The private-drop choice
is latched so a late MIME change or URI request cannot start an export after the
handoff. External receivers retain the accepted-drop URI delivery path.

The upstream MIT and Apache-2.0 notices are retained. Rust Commander distributes
this adaptation under the MIT option; packaged notices include its full license.
Ordinary builds require no downloads beyond Cargo's existing dependencies.
