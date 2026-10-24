# Next twelve features

[← Back to README](../README.md)

This batch keeps Hayro and extends Rust Commander's existing Slint desktop.
The checks and platform limits are recorded in [validation](../parity/VALIDATION.md).
It does not establish full muCommander parity.

## 1. Directory tree

**View → Directory tree** shows a second navigation sidebar. Expand/collapse
arrows load directories on a worker; selecting a name navigates the active pane.
The active location expands its folder and ancestors and scrolls into view, including hidden ancestors of the current location. With the tree
focused, Up/Down moves the cursor, Left/Right collapses/expands or moves to a
parent/child, and Enter opens the focused directory. Local, connected remote and
archive locations use the same tree. Local expansion does not follow symbolic links; remote link identification depends on provider metadata.
The tree has a 5,000-row and 64-level bound; hidden files follow the pane setting.

## 2. Native moves and renames

Moves to unused targets first try an exclusive native rename. This preserves
file identity and avoids copying file data to the destination on the same
filesystem. A cross-device result falls back to guarded copying/removal.
Single-item renames also use an exclusive native rename. Existing targets are
never replaced by this path; replacements and merges retain their existing
staging/recovery workflow. Undo still prepares recovery snapshots, so journaling
can involve substantial I/O even for a native move.

## 3. Extract into existing folders

The extraction confirmation accepts a new or existing destination. Existing
folders offer Stop, Skip, Keep both, Replace, optional folder merging and
per-file choices. Conflict previews inspect archive headers without inflating
the data. Applying validates and extracts into a private staging folder before
using the normal local transfer/recovery planner. Local Undo covers committed
changes. Traversal, links, special files and extraction-size limits remain
rejected. The original lower-level extraction API still requires a new folder.

## 4. Viewer search and byte navigation

The text/hex viewer has **Find next**, **Find hex** and a byte-offset field.
Text search is literal and case-sensitive, using the selected encoding. Hex
search accepts pairs such as `DE AD 00`; offsets accept decimal or `0x` notation.
Local searches stream across the entire file with bounded memory and can be
cancelled. Matches load the surrounding text page and select the matching text.
Editable documents search the current draft instead of discarding unsaved edits.
Hex byte navigation displays a bounded 64 KiB window. Remote/archive searches
retain bounded input limits; large-file paging and byte jumps require local files.

## 5. External editor save-back

**File → Edit externally…** prepares a private working copy, up to 32 MiB.
Choose an installed application or leave the application path blank for the
system default, then choose **Open editor**. Save in the external application,
return to Rust Commander, **Review changes**, and explicitly **Save back**.
Local files, remote files and entries in writable local ZIP/TAR/7z archives are
supported. Save-back refuses changed originals; remote saves keep recovery
copies and local saves/archive replacements participate in Undo. Nested and
remote archive containers remain read-only. Closing protects an open editor
session; **Discard working copy** explicitly removes it. A failed save retains
the working copy for review. Working copies are temporary, not crash recovery.

## 6. Application associations

**File → Open With…** includes a filterable installed-application list, an
explicit path field and a remembered filename pattern. **File → Application
associations…** adds/replaces/removes rules. Rules match filenames without case
sensitivity, in saved order, and apply to normal opening of local files.
F3/F4 retain the built-in viewer/editor. Programs receive filename arguments
directly without a shell. Discovery scans standard application locations with
bounded depth; applications outside those locations can be entered by path.

## 7. Checkpointed copies and live transfer controls

Local, local/remote and remote/remote copies offer **Checkpoint this copy** in the
confirmation, including folder merges, Skip, Keep both, Replace and per-file
choices. The durable plan fixes destination names and conflict decisions before
writing. A retry never chooses another Keep both name. Links are refused in this
mode; ordinary local copies can still preserve links.

The private batch manifest and staging files live under the configuration
folder's `transfers/` directory; credentials are excluded. OS locks prevent two
processes from replaying or discarding the same batch; empty sibling `.lock` files
remain after cleanup. Completed targets are
checked by content hash, partial local files against the source prefix, and remote
readers resume at the verified byte offset. Uploads append to hidden server staging
files when supported; other services restart that staged file. Verification can
reread substantial data.

Checkpointed replacements verify a local recovery copy before clearing an old
destination. Each deletion has a durable intent and completion receipt. Recovery
checks the remaining original files and rejects unexpected additions or changes.
Originals remain under `original-N` in the batch folder, with the matching target
in `manifest.json`. The result reports this folder even after success. Review and
restore originals manually if needed; queue Discard refuses to delete a batch
that holds verified originals. These batches do not advertise local Undo.
Batches without replacements remove local checkpoints after success. Hidden server
staging files can require explicit cleanup. Failed/cancelled checkpoints and
replacement originals can occupy the full size of the transferred trees.

**Pause** beside the progress bar pauses the active transfer; **Resume** continues
it. **Pause queue** only holds waiting jobs. Pausing is cooperative between chunks
and provider calls; an in-flight OS/server call may finish before **Paused** is
shown. Cancel also works while paused or waiting to retry.

**Transfer queue and history…** exposes a live bandwidth limit in KiB/s
(`0` = unlimited) and the automatic retry count (`0` = disabled, maximum `8`,
default `3`). Both settings are saved and shared across windows. The limit paces
streamed copy, staging and checkpoint verification work. Provider buffering and
single-request commits can still produce short network bursts. Resuming or
changing the limit does not accumulate unused bandwidth credit.

Only checkpointed copies automatically retry typed temporary I/O/provider errors,
HTTP 429 and server errors, using delays of 1, 2, 4, 8, 16 and then 30 seconds.
Authentication, permissions, changed sources/destinations, corrupt receipts and
conflicts stop for review. Ordinary uncheckpointed operations and moves are not
replayed automatically. Interrupted work can also be retried manually from the
queue; restored queues start paused.

SMB and filesystem adapters publish with exclusive native rename. Other providers
use their normal exclusive-create support; providers without it retain the
concurrent-write limitation. An interrupted final publish can require review of
a partial final target. Moves continue to use the guarded move workflow.

## 8. Recursive remote conflicts

Enable **Merge existing folders** for ordinary remote transfers. Preview and
execution descend through existing directory pairs and apply choices to each
conflicting file. Plans are checked before writing. Replacements retain recovery
copies. A move with any skipped descendant keeps its original top-level folder
intact, including already-copied children. Links are refused; provider operations
without conditional writes cannot exclude simultaneous external changes.

## 9. Remote synchronization

**Operations → Synchronize** accepts local or connected remote locations in
either pane, including two remote profiles and locations opened directly by URI.
The dialog retains the actual pane locations instead of reparsing their public
display addresses. Editing a root or direction cancels obsolete previews.
Preview compares contents and produces reviewable per-item directions and
explicit conflict choices. Read-only destinations, including HTTP, cannot be
selected for writes; a queued plan is checked again before any write.

Deletions remain opt-in and separately confirmed. Applying checks the root and
item content snapshots and refuses stale plans. Replacements/deletions preserve
recovery copies. Mixed/remote plans do not offer a partial local Undo. Remote
scans can involve substantial network I/O; review is bounded to 5,000 differences.
Protocol authentication and live-provider limits still apply.

**Compare folders** also works across local/local, local/remote, and remote/remote
panes. It recursively hashes file contents with SHA-256, including files whose
listing sizes are unknown or rounded. This can download every compared remote
file. Links are reported without following them; archives are not comparison
roots. Traversal is bounded to 64 levels and 100,000 entries and supports Cancel.
Comparison reports recovery files as ordinary entries; synchronization excludes
its own recovery copies. The [advanced tools guide](ADVANCED_TOOLS.md) covers
the newer include/exclude rules, shared presets, optional unverified metadata
comparison, read-only text differences and filtered synchronization.

## 10. Native copy metadata

Local copies preserve mode/mtime plus macOS ACLs, extended attributes, Finder
metadata and resource forks; Linux user attributes/POSIX ACLs; and Windows DACLs.
Unsupported privileged Linux security/trusted namespaces and Windows alternate
data streams/ownership are outside this addition. Native move/rename retains
metadata directly. Metadata-copy errors are reported instead of silently
claiming preservation. New Undo records guard native metadata as well as file
contents; older journal fingerprints remain compatible.

## 11. Restore all windows

The saved session includes every open window's pane locations, tab titles/locks,
active tabs/pane, split layout, window size and screen position (when supported). Ordinary startup restores that
set; explicit CLI paths start the requested window instead. Closed windows are
removed from the next saved set, while quitting preserves the full open set.
Up to sixteen windows can be open; restored positions outside available monitors
fall back to the desktop's normal placement. Editor drafts, terminals, passwords and temporary archive cache paths are not
serialized. The existing shared queue/preferences runtime remains application-wide.

## 12. Hayro PDF controls

Encrypted PDFs open with a password prompt. The viewer adds go-to-page,
clockwise rotation and fit-width, with worker-thread rendering. A per-document
LRU retains at most three unrotated images and 24 MiB of page pixels; rendered
resolution and input limits remain bounded. Passwords/cache data stay in memory.
This adds viewing controls, not PDF text search, annotation editing or forms UI.
