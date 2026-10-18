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

Move and single rename currently use copy-then-remove. Originals are removed
only after copying and metadata checks complete. Batch rename uses native
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

NRG, CPIO, and RPM still need dedicated format fixtures. Format-edge-case
coverage is incomplete; see the [validation record](../parity/VALIDATION.md).

Double-click a supported archive to browse it, then use **F5** to extract selected
entries into a new folder. Archive tools edits local ZIP/TAR/7z containers (Add/Remove/Rename),
creates AES-encrypted ZIP/7z archives, and unlocks existing encrypted archives.
Nested and remote archives are materialized into a bounded temporary cache and
remain read-only. ZIP mutation preserves unchanged compressed entries and checks
the original before replacement. Encrypted ZIP edits repack with encryption after
unlocking; new files are encrypted too. TAR/7z edits use guarded extraction/repacking.
See the [feature guide](FEATURES_0.5.md) for limits and local Undo behavior.

Extraction requires a new folder and rejects absolute paths, parent traversal,
Windows drive/backslash paths, links, and special files. It never merges into
an existing folder. Limits are **100,000 entries** and **10 GiB of declared
uncompressed data**. Failed extraction removes its new output folder.

Archive creation rejects symlinks and outputs inside a selected source directory.
Stream formats accept one regular file.

## Viewers and text editing

| Tool | Current support and limits |
| --- | --- |
| Text viewer/editor | UTF-8, UTF-16 LE/BE, Windows-1252 and ISO-8859-1, up to 2 MiB |
| Large local text viewer | Read-only 256 KiB pages, with selectable encoding |
| Image viewer | Input up to 32 MiB; image dimensions are bounded |
| PDF viewer | Input up to 32 MiB; page navigation and bounded output resolution |
| Hex viewer | Displays at most the first 512 KiB |
| Checksums | SHA-256, SHA-512, SHA-1, and MD5 |

Text saves preserve encoding, BOM and CRLF line endings, atomically replace the
file and refuse changed contents. The save-encoding choice allows conversion;
unrepresentable legacy characters are rejected. Invalid UTF-8 opens as hex and
can be decoded explicitly. Large local files use bounded read-only pages;
remote/archive text editing remains limited to 2 MiB.

Text saves atomically replace the file and refuse changed contents. Symlinks,
and archive entries are view-only. Remote encoded text entries support guarded saves
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
