# Rust Commander 0.5 feature guide

[← Back to README](../README.md)

**Mod** means Cmd on macOS and Ctrl on Windows/Linux. This batch adds the next
12 features; it does not establish full muCommander parity.

| # | Feature | How to use it |
| --- | --- | --- |
| 1 | Recovery storage manager | **Operations → Recovery storage** shows disk use, recovery copies and incomplete journals. Set retention targets, preview cleanup, pin records, or enable automatic cleanup. |
| 2 | Redo and broader Undo | **Mod+Z / Mod+Shift+Z** undo/redo eligible local operations, including text saves, permissions and modification times. History also offers per-record controls. External changes or altered backups cause refusal. |
| 3 | Persistent transfer queue | **Mod+J** opens saved work. Interrupted queues reopen paused; review source/destination details, then resume, retry or discard. Single regular-file local copies/moves using the Stop conflict policy retain verified checkpoints. |
| 4 | Directory synchronization | **Operations → Synchronize folders** previews left-to-right, right-to-left or two-way changes. Resolve conflicts explicitly; deletion choices require enabling deletions and confirming their count. Apply rejects stale plans. |
| 5 | Remote text editing | **F4** edits remote UTF-8 files up to 2 MiB. Save checks the original contents, keeps a server-side recovery copy and verifies the upload. |
| 6 | Direct SMB browsing | **File → Connections → smb** connects directly to SMB2/3 shares without mounting them. Browsing, streamed upload/download, rename, mkdir, deletion, search and editing use the shared remote workflows. |
| 7 | Remote search | **Operations → Find files** works in remote folders with filename globs, optional content text, hidden-file controls and cancellation. |
| 8 | Broader archive editing | **File → Archive tools** adds/removes/renames entries in local ZIP, TAR, TAR.GZ/BZ2/XZ and 7z containers. Unlock encrypted archives first; adding files to an encrypted ZIP preserves encryption. |
| 9 | Preview and Quick Look | **Mod+P** toggles a persistent text/image/hex/PDF preview pane. **Mod+Shift+P** launches macOS Quick Look for local selections; other platforms use the built-in preview. |
| 10 | Native file actions | **File** and the row context menu offer **Open With**, **Reveal** and **Show package contents**. Double-clicking a macOS `.app` launches it. Open With accepts an application path and lists `/Applications` on macOS. |
| 11 | Native drag-out and broader drops | **Alt+drag** or **Selection → Native drag-out** exports local files. On Linux a native GTK window presents the files to drag into another application. Drops into remote folders or editable archives open the upload/archive confirmation controls. |
| 12 | Named workspaces | **Navigate → Workspaces** saves, opens, renames and deletes named sets of both panes, tabs, active locations and layout preferences. Credentials remain in the OS store. |

Temporary nested/remote archive tabs restore their persistent containing location;
reopen the nested archive after switching or restarting. Workspace names are
unique, and loading a workspace does not retarget an operation already running.

## Recovery and queue details

Recovery retention defaults to **2 GiB / 30 days**, with automatic cleanup off.
These are cleanup targets: pinned chains and the latest reversible chain are
protected and can exceed them. Cleanup expires whole Undo/Redo chains before
removing their backups. History labels/outcomes remain. Incomplete journals are
kept for manual inspection; **Open recovery folder** opens their location.
Recovery copies contain original file contents and consume real disk space.

Queue recipes are saved atomically in `queue.json` beside settings. Saved recipes
require UTF-8 file paths. Passwords,
tokens, original editor buffers and unsaved drafts are excluded. Restored jobs
that need these inputs stop for review; cancel the recipe and use **Retry / reopen**
to supply the inputs again. Failed/interrupted recipes are retained until retried
or discarded. Hover a recipe for its paths. Retry other transfer types restarts
the operation and rechecks conflicts; it does not promise protocol-level resume.

Resumable local file copies use private `transfers/` checkpoints and source
witnesses. The worker verifies the full source and the saved prefix before
appending; changed sources, corrupt checkpoints or newly occupied destinations
are refused. Discarding the recipe removes its checkpoint. A crash after output
was committed may leave a recipe requiring manual review of the existing output.

Undo/Redo covers journaled local changes. It does not reverse remote writes,
permanent deletion, Trash operations, arbitrary external programs or every
upstream action. A new local mutation clears the Redo branch.

## Synchronization safeguards

Synchronization currently accepts **two local, non-overlapping folders** and
compares file contents. Links and special files are refused. Two-way mode copies
one-sided entries and starts conflicts at Skip; without a historical baseline it
does not infer which side deleted a file. Deletions are explicit choices.

The plan records directory/file fingerprints and checks them before execution,
after recovery preparation and before replacement. The affected roots are
journaled so cancellation/partial results can be undone. Detected external changes
during execution disable automatic Undo for that record and retain recovery copies
for manual review, avoiding restoration over the other client’s changes. This can require disk
space comparable to the roots being changed. Keep the recovery directory outside
the synchronization roots. Limits are 100,000 entries; this is not a remote sync
or scheduled backup engine.

## Remote safeguards

For SMB, use an endpoint such as `smb://nas.local:445`, enter **Share**, and set
**Root folder** to a path inside it. Domain users can enter `DOMAIN\user`.
Credentials use the same memory/OS-store policy as other remote connections.
The pure Rust SMB adapter does not require a system SMB mount or libsmbclient.

Remote search is bounded to 100,000 visited entries, depth 256 and 10,000 results.
Content matching reads files up to 2 MiB; larger/unreadable entries are reported.
Cancellation takes effect between requests; a request can take up to its timeout.

Remote text saves retain `.rust-commander-edit-*.bak` originals on the server.
Services supporting conditional writes use an ETag condition. SMB and services
without that capability have a guarded content check followed by a write, which
cannot exclude an external writer in the gap. Upload failure can leave partial
output; retain the recovery copy until the result is reviewed. Remote saves do
not enter local Undo/Redo. Cached/nested archives remain read-only.

## Archive and desktop limits

TAR, 7z and encrypted ZIP editing extracts into a private working tree, applies
the edit, validates a newly packed container and replaces the original only after
checking it again. Existing extraction limits apply: 100,000 entries / 10 GiB.
Repacking can change compression, entry ordering and container-specific metadata;
it preserves encryption for encrypted ZIP files. Unencrypted ZIP edits retain the
existing raw-copy path for unchanged entries. RAR/ISO/other reader formats and
single-file compression streams remain read-only for in-place editing.

Linux drag-out uses a separate GTK4 event loop, so it works independently of the
main winit window's X11/Wayland backend. Drag from that helper window; it is a
two-step workflow. Linux builds require GTK4 development files, and distributed
executables require the GTK4 runtime. Native drag/drop into other desktop apps,
Windows integration and real Wayland sessions still need interactive validation.
Open With uses argument arrays and never interprets filenames as shell commands.

## Reproducing optional integration checks

The normal test suite needs no remote credentials. The optional SMB test expects
a **disposable**, loopback-only Samba share named `commander`, with test account
`commander` / `commander-test-only`. It creates a uniquely named folder and removes
that folder after success. Do not point it at a production account.

```sh
RUST_COMMANDER_TEST_SMB_ENDPOINT=smb://127.0.0.1:1445 \
  ./scripts/cargo-local.sh test --locked --test features_05 \
  smb_live_list_stream_upload_download_rename_edit_search_delete -- --ignored --exact
```

On Linux, the native drag-source test can run with a disposable Xvfb display:

```sh
xvfb-run -a cargo test --locked --lib \
  gtk_native_drag_source_exposes_file_list -- --ignored --test-threads=1
```

See [validation evidence](../parity/VALIDATION.md) for what was actually exercised.
