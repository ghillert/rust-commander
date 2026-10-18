# Rust Commander 0.4 feature guide

[← README](../README.md)

This release implements the twelve requested feature areas. The table records
behavior and its practical limits; it is not a claim of full muCommander parity.

| # | Feature | Use and current scope |
| --- | --- | --- |
| 1 | System file clipboard and drag-out | **Mod+C/X/V** exchanges local files with the system clipboard. Copy/Cut remote selections works between panes. **Alt+drag** starts native file drag-out on macOS/Windows. Linux offers clipboard fallback; native drag-out for the winit window backend remains open. |
| 2 | Undo and recovery history | **Mod+Z** undoes the latest eligible local operation. History provides individual Undo actions. Local moves, renames, copies, replacements, creates and archive edits capture recovery copies. Complete contents and metadata are checked before restoration; external changes cause refusal. Remote operations, Trash, permanent deletion, text saves, permissions and split/combine have no Undo. No Redo. |
| 3 | Configurable columns and toolbar | **Settings → Columns / Toolbar actions** adds, removes and reorders controls. Resize column headers by dragging their boundaries. Name remains first. Available columns include size, modified time, extension, type and local Unix permissions. Preferences persist across restarts. |
| 4 | ZIP editing | **Operations → Archive tools** adds files, removes selected entries or renames entries in local ZIPs. F2 and Trash inside a local ZIP open the corresponding confirmation. F5 copying local files into a ZIP opens Add files. Changes rebuild a temporary archive before replacement and can be undone. Nested and remote archive caches stay read-only. |
| 5 | Drives and breadcrumbs | The sidebar lists mounted drives. Click a breadcrumb above a local pane to navigate to an ancestor; the editable address remains available. Linux removable mounts are discovered under `/media`, `/mnt` and `/run/media`. |
| 6 | Queue and history | **Mod+J** opens the queue and operation history. Submit another operation while one runs to queue it. Queued work and active work can be cancelled. Completed, failed, cancelled and undone records persist, including completed workers whose result was not yet received by the UI. Pending queue entries do not survive restart. |
| 7 | Per-file conflicts | Transfer confirmation previews existing target paths and sizes. Select Skip, Keep both, Replace or Stop for individual conflicts. The global policy covers remaining items; **Apply to all** clears individual overrides. Local folders can merge; remote conflicts apply to whole top-level items. |
| 8 | Encrypted ZIP and 7z | Archive tools creates AES-encrypted ZIPs and 7z archives from local selections. Choose Unlock archive for existing encrypted archives; passwords remain in memory. Wrong passwords are checked by reading encrypted data. ZIP filenames remain visible; newly created encrypted 7z archives also encrypt headers. Adding plaintext entries to an encrypted ZIP is refused. |
| 9 | Nested archives | Double-click a supported archive inside another archive. A private temporary copy is browsed, and Parent returns to its containing archive. Limits: eight nesting levels, 128 MiB per materialized archive, 512 MiB session cache. The cache is cleared on normal exit. |
| 10 | Remote file workflows | Remote rename, read-only archive browsing and conflict policies use the same pane workflow. Rename streams a copy and removes the source only after metadata checks. Replacements retain hidden recovery copies at the destination. These operations are not atomic remote transactions. |
| 11 | Browser cloud sign-in | Google Drive, Dropbox and OneDrive profiles offer browser authorization with PKCE, a validated loopback callback and automatic token refresh before requests. Register a desktop OAuth client first; see [OAuth setup](OAUTH.md). Account policy, consent, keychain prompts and live provider behavior still need validation. |
| 12 | Localization | Settings selects system language, English, German or Spanish. Main menus, preferences, dialogs, help and common application errors are translated. Paths, user content and detailed OS/provider diagnostics retain their original text. Native predefined menu items follow the OS language. Additional languages can extend `locales/`. |

## Recovery storage

Operation records and local Undo snapshots live in `history/` beside
`settings.json`. Passwords and queued operation parameters are not serialized.
The journal uses atomic record writes and also recovers completed per-operation
records when the index is absent or damaged. A damaged index is preserved when
rewritten. This is recovery for recorded operations, not crash-atomic filesystem
transactions; a process killed during an operation can leave partial output and
backups requiring inspection.

Undo snapshots retain full original data and can consume substantial disk space.
There is no automatic expiration. After reviewing files and when Undo is no longer
needed, close Rust Commander before removing its `history/` directory. Recovery
folders named `.rust-commander-replaced-*` live at transfer destinations and are
independent of the journal; review them separately before removal.

A failed or cancelled operation can have an Undo action if it changed local files.
Undo checks every recorded destination and recovery copy before changing anything,
stages restoration, and rolls back on failure. External edits are never silently
discarded. Concurrent filesystem writes are not locked out.

## Platform and service validation

Automated tests use disposable folders, archive fixtures, a filesystem remote
adapter and loopback HTTP servers. They cover recovery, mixed conflicts, ZIP
mutation, encrypted ZIP/7z, nested/remote archives, OAuth protocol handling,
preferences, localization and clipboard URI parsing. An explicitly invoked,
ignored-by-default test checks the real desktop clipboard while preserving its
prior contents in memory.

Only the available Apple Silicon Mac has been exercised interactively. CI covers
builds/tests on macOS, Windows and Ubuntu; those desktop interactions still need
platform testing. Linux clipboard support uses X11 or the optional Wayland data
control protocols supported by the compositor, with X11 fallback. Native Linux
drag-out is not implemented. Live cloud login, refresh, FTP/SFTP/S3 and keychain
prompts have not been validated with accounts or test servers.
