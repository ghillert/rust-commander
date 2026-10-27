# Recovery, archive save-back and desktop conveniences

[← Back to README](../README.md)

## Recovery manager

Choose **Operations → Recovery manager…** to review retained originals from
Undo/Redo history, checkpointed replacements and indexed local/remote backups.
The list shows original names, backup locations and cleanup scope. Select rows
individually; clicking a selected row unselects it. Changing the filter clears
the selection and permanent-cleanup confirmation.

**Restore selected copies** restores up to 100 selected originals into the
displayed destination folder. You can change that folder before restoring.
The original parent folder is suggested for a single selection. Existing files
are kept: a conflicting `report.txt` becomes `report (1).txt`, for example.
Recovery originals remain available after restoration. Local restores participate
in Undo; remote restores use the existing provider write guards. Cancellation or
a later error can leave already restored copies in place, with a reported count.
An operation restored from the saved queue requires renewed review.

**Browse backup folder** opens the selected backup's parent in a pane. This also
allows normal browsing and copying of individual files inside a retained folder.
Remote credentials are obtained only when accessing a remote backup; discovery
reads the local index without connecting to every provider.

For a replacement folder created before the index existed, enter its
`.rust-commander-replaced-*` path and choose **Import older recovery folder**.
Import recognizes the existing `RECOVERY.txt` marker and `items/` layout and
infers the original destination from the recovery folder's parent. It does not
move or remove any files. Import is bounded to 5,000 inspected entries/64 levels.

**Delete reviewed backup groups** requires selecting eligible originals and
typing `DELETE`. The dialog explains the scope before enabling that button:

- An Undo/Redo group expires the complete operation chain. Pinned chains and
  the latest reversible chain are protected.
- A checkpoint group removes the complete finished batch and all of its
  retained originals. Incomplete, locked, corrupt or unrecognized batches are
  preserved for review.
- An indexed ordinary replacement removes that retained original only.

Cleanup never removes the current destination files. Recovery snapshots are
checked again before restoring or cleaning; changed copies are kept for review.
The existing **Retention settings…** page controls automatic cleanup of eligible
history chains. Indexed ordinary/remote backups and checkpoint originals require
explicit cleanup through the manager.

The index is stored under the configuration folder's `recovery/items.json`,
with an OS lock and atomic replacement. It contains locations and fingerprints,
not credentials. Discovery is bounded to 10,000 items; checkpoint manifests to
32 MiB and the index to 4 MiB. Already deleted local backups are omitted.

## Nested and remote archive editing

Normally opened ZIP, TAR, compressed TAR and 7z containers support guarded
Add/Remove/Rename through **Archive tools…**. F2/Trash inside an editable archive
open the corresponding archive confirmation. F5 copying local files into an
editable archive opens Add files. Drag/drop and reviewed external-editor
save-back use the same container publication path.

Changes are prepared in private working files. A nested edit rebuilds its
containing archives outward, then publishes the outer local or remote source.
The source must still match the snapshot from opening; an external change stops
save-back. A failed preflight or cancellation before publication preserves the
original containers. The cached view refreshes only after the source commits.
Other cached siblings whose parent changed must be reopened before editing.

Local publication journals the outer source for Undo and preserves native
metadata. Remote publication verifies a server-side recovery original before
uploading, verifies the uploaded contents and records the backup in the recovery
manager. Remote changes do not advertise local Undo. Providers without
conditional writes cannot exclude a simultaneous external writer atomically;
an interrupted upload can require reviewing the retained server backup.

Limits remain eight nested levels and 128 MiB per edited container, with at most
512 MiB in the materialized archive cache. Existing extraction entry/data limits
also apply. HTTP archives, unsupported formats and **Open As** format overrides
remain read-only. Passwords must be available for encrypted containers and stay
in memory. Cached archive edit recipes cannot rebuild their source session after
an application restart; reopen the source archive to repeat the operation.

## Convenience commands

- **Selection → Copy names without extensions** copies one name per line.
  Files lose their final extension (`archive.tar.gz` becomes `archive.tar`);
  directory names and leading-dot names such as `.profile` are preserved.
- **View → Show parent directory row** toggles the `..` navigation row and saves
  the preference. This row cannot be marked or included in file operations.
  The existing parent toolbar button and Backspace shortcut remain available.
- **Window → Switch to window 1–10** uses `Mod+Alt+1` through `Mod+Alt+0` by
  default, where Mod is Command on macOS and Control elsewhere. Slots follow
  current open-window order; a missing slot does nothing. Bindings can be changed
  in Preferences, and upgrades preserve existing custom shortcut assignments.

## Updates, diagnostics and bug reports

**Help → Check for updates…** checks GitHub's latest stable published release
on a worker. It handles an unpublished first release, offline/error responses and
rate limits. Checks are manual, with a 20-second request timeout and 1 MiB response
bound. The dialog compares versions and opens the verified repository release
page. It does not download, install or restart the application.

**Help → Diagnostics…** displays a copyable version/platform/build summary.
It excludes local paths, filenames, credentials, connection profiles and
operation history. **Help → Report a bug…** shows that same summary and offers
to open a prefilled issue in the browser. Review/edit it there before submitting;
the application does not post a report automatically. Additional logs and
screenshots are attached manually.

The remaining Help links, Donate, Email files, bookmark explorer, bring-all-forward
and debug console are described in [desktop commands](DESKTOP_COMMANDS.md).

Implementation/fixture evidence and native platform limits are recorded in
[the validation record](../parity/VALIDATION.md).
