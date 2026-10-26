# File operations and format support

[← Back to README](../README.md)

This guide describes the current local transfer, archive, and viewer behavior.
For remote transfers, see [Remote connections](REMOTE_CONNECTIONS.md). The
[parity checklist](../parity/PARITY.md) records remaining compatibility work.

## Local transfers

Local transfers stop on existing targets by default, including dangling symlinks.
The confirmation dialog also offers skipping conflicts, keeping both with numbered
names, or replacing targets. Folder merging is optional. Conflicts are planned
before a batch starts. Folder transfers into themselves are refused,
including through symlinks. Local symlinks are preserved rather than followed.
Special files such as FIFOs are unsupported. Local output files use temporary
files and a commit that refuses existing targets. Permissions and modification
times are preserved for local file copies.

Moves to unused targets and single renames use exclusive native rename where possible.
Cross-device moves fall back to copy-then-remove with source checks. Undo still
prepares recovery snapshots; see the [next feature guide](FEATURES_NEXT.md). Batch rename uses native
exclusive renames through a staging folder and supports cycles. A rollback
collision preserves staging files and reports their recovery location.

### Replacement and recovery

Replacements are staged completely before an existing target is moved into a
hidden `.rust-commander-replaced-*` recovery folder in the destination. The result
reports its path, and `items/` preserves the original relative paths. Review these
backups before removing them. Folder replacement without merging preserves the
whole original folder there.

When a move skips a nested conflict, its entire source folder is retained,
including already copied children. Remote transfers also offer per-item conflict
choices; the lower-level `Copy`/`Move` API refuses existing targets. Cancelled or failed batches
can keep completed copies. Cancellation is cooperative and cannot interrupt an
OS call already in progress.

These are desktop operations, not filesystem transactions; concurrent external
changes are not locked out.

### Live transfer controls and durable copies

The progress bar offers **Pause** / **Resume** for active transfers. The queue
and history dialog holds the bandwidth limit and automatic retry settings.
Select **Checkpoint this copy** to recover a whole local or remote batch with
fixed merge/conflict decisions. Checkpointed replacements retain originals in
the private batch recovery folder reported by the result, rather than the ordinary
replacement folder described above. See [checkpoint storage, retry rules and
provider limits](FEATURES_NEXT.md#7-checkpointed-copies-and-live-transfer-controls).

## Archives

| Format | Browse / extract | Create |
| --- | --- | --- |
| ZIP | Yes | Yes |
| TAR | Yes | Yes |
| TAR.GZ / TGZ | Yes | Yes |
| TAR.BZ2 / TBZ2 | Yes | Yes |
| TAR.XZ / TXZ | Yes | Yes |
| 7z | Yes | Yes |
| GZip / BZip2 / XZ | Yes, as single-file streams | Yes, one regular file |
| RAR | Yes, through libarchive | No |
| ISO / NRG | Yes, through libarchive | No |
| AR / DEB | Yes, through libarchive | No |
| CPIO / RPM | Yes, through libarchive | No |

Mark local files/folders with Space, then choose **Pack files…** from the right-click
menu or **Operations → Pack files…**. The dialog mirrors the muCommander workflow:

- **Add selected files to** suggests a name in the opposite local pane (or the
  current folder when the opposite pane is remote). Edit it to choose another path.
- **Archive format** selects ZIP, 7z, TAR, compressed TAR, or a single-file stream.
  Changing format replaces the complete archive suffix, including `.tar.gz`.
- **Compression level** offers the encoder default or an explicit level: ZIP/GZip
  and 7z/XZ accept 0–9; BZip2 accepts 1–9. The same choices apply to compressed
  TAR variants. ZIP level 0 stores files, GZip level 0 skips compression, and 7z/XZ
  level 0 uses the fastest compression preset. Plain TAR is uncompressed.
  Higher levels usually trade more time and memory for smaller archives. Switching
  formats retains a supported level and otherwise resets it to Default. The selected
  level survives queue recovery and is also available when creating encrypted archives.
- **Comment** stores a UTF-8 archive comment for ZIP, up to 65,535 bytes. Other
  formats hide this field; switching back to ZIP restores the draft comment.
- **Run in background** keeps the file panes available during packing. Otherwise
  a cancellable progress dialog appears; **Run in background** is also available
  there. Background jobs use the existing durable operation queue and history.
- The **file(s)** disclosure expands the frozen list of source paths. Cancelling
  the dialog creates no archive and preserves the source marks.

**Pack** creates a new archive and refuses to overwrite an existing destination.
The selected format determines the output suffix; a missing/different suffix is
corrected before packing. For password-protected ZIP or 7z, use **File → Archive
tools… → Create encrypted archive**.

NRG, CPIO, and RPM still need dedicated format fixtures. Format-edge-case
coverage is incomplete; see the [validation record](../parity/VALIDATION.md).

Double-click a supported archive to browse it, then use **F5** to extract selected
entries into a new or existing folder, with normal conflict choices and optional merging. Archive tools edits local ZIP/TAR/7z containers (Add/Remove/Rename),
creates AES-encrypted ZIP/7z archives, and unlocks existing encrypted archives.
Nested and remote archives are materialized into a bounded temporary cache and
remain read-only. ZIP mutation preserves unchanged compressed entries and checks
the original before replacement. Encrypted ZIP edits repack with encryption after
unlocking; new files are encrypted too. TAR/7z edits use guarded extraction/repacking.
See the [feature guide](FEATURES_0.5.md) for limits and local Undo behavior.

Extraction stages into a new private folder before merging into an existing destination.
It rejects absolute paths, parent traversal,
Windows drive/backslash paths, links, and special files. Existing-folder merges use the local transfer/recovery planner. Limits are **100,000 entries** and **10 GiB of declared
uncompressed data**. Failed extraction removes its new output folder.

Archive creation rejects symlinks and outputs inside a selected source directory.
Stream formats accept one regular file.

## Viewers and text editing

| Tool | Current support and limits |
| --- | --- |
| Text viewer/editor | UTF-8, UTF-16 LE/BE, Windows-1252 and ISO-8859-1, up to 2 MiB |
| Large local text viewer | Read-only 256 KiB pages, with selectable encoding |
| Image viewer | Input up to 32 MiB; image dimensions are bounded |
| PDF viewer | Input up to 32 MiB; password unlock, page navigation, rotation, fit-width, text search/selection and bounded page cache |
| Hex viewer | Initially the first 512 KiB; local byte jumps/search display bounded 64 KiB windows |
| Checksums | SHA-256, SHA-512, SHA-1, and MD5 |

**File → Open As…** (also in the right-click menu) overrides a file's extension
for text, hex, image, PDF or archive interpretation without renaming it. PDF
pages support mouse selection/copying, a selectable Text view, and cancellable
case-insensitive document search with previous/next and wrapping. See
[Open As and PDF text tools](VIEWERS.md) for workflows and limits, including
read-only archive overrides and PDFs that need OCR.

Text saves preserve encoding, BOM and CRLF line endings, atomically replace the
file and refuse changed contents. The save-encoding choice allows conversion;
unrepresentable legacy characters are rejected. Invalid UTF-8 opens as hex and
can be decoded explicitly. Large local files use bounded read-only pages;
remote/archive text editing remains limited to 2 MiB.

Text saves atomically replace the file and refuse changed contents. Symlinks
and archive entries are view-only in the built-in editor; reviewed external-editor
save-back supports writable local archive entries. Remote encoded text entries support guarded saves
with server-side recovery copies; services without conditional writes cannot
exclude simultaneous external writes atomically. Local saves are journaled for Undo/Redo. Use **Open externally** for
other formats. Additional text encodings, image animation, and full PDF compatibility
remain part of the [roadmap](../README.md#roadmap).

## System Trash

**File → Open Trash** opens Finder Trash, Windows Recycle Bin or the Linux
`trash:///` location. **Empty Trash…** requires typing `EMPTY` and an idle
transfer queue. It empties the system-wide Trash, including other applications'
files, and cannot be undone through Rust Commander's recovery journal. It is
never automatically resumed from a persisted queue. macOS uses Finder automation
and may request permission; Windows uses its native Recycle Bin API; Linux needs
`gio` and a working GVfs Trash backend. Native emptying has not been executed as
part of automated validation.
