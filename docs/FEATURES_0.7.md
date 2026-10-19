# Rust Commander 0.7: twelve parity additions

[← Back to README](../README.md)

This guide describes the preceding feature batch. The [next feature guide](FEATURES_NEXT.md)
supersedes the window-restoration, viewer-search and related limitations below.

This release expands the Slint desktop workflow. It does not establish full
muCommander parity; see the [checklist](../parity/PARITY.md) and
[validation record](../parity/VALIDATION.md). **Mod** means Cmd on macOS and Ctrl
on Windows/Linux. Menu commands can be assigned primary and alternate shortcuts.

## 1. Type-ahead quick find

Focus the file list and start typing a filename prefix. Quick find moves the
cursor without changing the filter or marked files. Matching is case-insensitive
and normalizes composed/decomposed Unicode names. Mod+G and Mod+Shift+G cycle
matches; Backspace edits the prefix and Escape clears it. The transient prefix
expires after a pause. Navigate → Quick find starts an explicit search.

## 2. Advanced selection

Selection → Select/Unselect by pattern accepts wildcard patterns separated by
semicolons, with optional case sensitivity. It affects visible files only and
validates every pattern before changing marks. Select same extension uses the
current file's extension. Block commands move or mark five rows, matching the
upstream block increment. Shift+arrows, Page Up/Down and Home/End extend marks
across every intermediate row. Page movement follows the visible list height.

## 3. Persistent tab controls

Navigate contains tab locking, title editing, reordering and moving a tab to the
other pane. A locked tab keeps its location: attempts to navigate open an unlocked
tab instead. Titles and lock locations survive restart and named workspaces.
A blank title restores the folder-derived label. The existing limit is 32 tabs
per pane. A temporary nested/remote archive location restores its persistent
containing location rather than a cache path.

## 4. Pane layouts

View → Side-by-side panes / Stacked panes changes orientation. Drag the divider
to resize the panes; Equal pane sizes resets the split. The ratio is saved and
bounded to keep both panes usable. Single-pane mode continues to show the active
pane and routes commands and incoming drops there. Layout preferences are shared
by open windows and included in workspaces.

## 5. Searchable quick lists

Mod+Shift+L opens a combined quick list. Navigate also offers dedicated lists for
recent locations, bookmarks, tabs, roots and recent files. Type to filter, use
Up/Down and Return to open, and Escape to cancel. Recent locations/files are
bounded to 50 distinct entries, newest first, and persist. Missing recent files
produce an error rather than silently opening another file. Window and imported
command lists use the same searchable interface.

## 6. Cross-pane navigation

Navigate → Open in both panes opens the chosen folder or supported archive in
both panes. Parent in both panes moves each pane up independently; Parent in other
pane leaves the active pane alone. These commands honor locked tabs. Ordinary
file viewing and file transfers still use their existing explicit actions.

## 7. Command bar and shortcuts

Settings offers up to twelve named command-bar button choices, in display order.
Choose None to remove a slot. Duplicate/unsupported entries and conflicting
shortcuts are rejected before applying. Find a shortcut filters the command
catalog; primary and alternate bindings support navigation and operations.
Leaving a binding blank disables it. Menus and command buttons display the current
binding. Default upgrades preserve customized bindings and avoid stealing keys.
Text fields retain their native editing shortcuts; editor Save has its own configurable binding, while terminal control bytes
remain appropriate to the focused terminal.

## 8. Metadata and automatic column sizing

Settings can show owner, group and permission columns. Clicking their headers
sorts by their metadata. Local Unix ownership is displayed as numeric UID/GID;
permissions use the local mode bits. Unavailable Windows/remote metadata displays
an em dash. View → Fit columns to content estimates widths from loaded rows and
toggles fitting them to the pane width. Column dragging adjusts proportional
widths when automatic sizing is enabled. Exact native ownership-name resolution
and ACL display are outside this addition.

## 9. Text encoding and large-file paging

The editor supports UTF-8, UTF-16 LE/BE, Windows-1252 and ISO-8859-1. BOM-marked
Unicode is detected. Decode encoding reinterprets the original bytes; it is
blocked while edits are unsaved. Save encoding converts on save and refuses
characters a legacy encoding cannot represent. Encoding, BOM and CRLF endings
are preserved by default. Local Undo restores the exact original bytes.

Files up to 2 MiB support editing, including guarded remote saves. Larger local
text files open read-only in 256 KiB pages with previous/next controls. UTF-8 and
UTF-16 page boundaries do not split characters. The hex view is bounded to the
first 512 KiB. Paging does not provide whole-file search or large-file editing;
remote/archive input limits remain in force.

## 10. System Trash actions

File → Open Trash opens the desktop's Trash. Empty Trash… requires
an idle queue and the literal confirmation EMPTY. It permanently removes the
system Trash contents, including files from other apps, with no Rust Commander
Undo. Persisted empty-Trash recipes require fresh confirmation.

macOS delegates to Finder and can request automation permission. Windows uses
Explorer and SHEmptyRecycleBinW. Linux uses GIO/GVfs (`libglib2.0-bin` and
`gvfs-backends` on Ubuntu/Debian). Emptying the real Trash is intentionally not
executed by the test suite; native platform validation remains outstanding.

## 11. Independent windows

Mod+N creates a window; Mod+Shift+W closes it. Window offers cycling and a
searchable window list. Each window owns its panes, tabs, filters, text draft and
terminal. Windows share one queue, active operation, recovery history, preferences
and saved collections. A transfer's completion belongs to its originating window;
closing an unrelated window does not cancel it.

Closing a window protects its dirty editor or owned operation. Application Quit
checks all windows and asks before discarding drafts or cancelling work. Pending
queued work remains persisted. The last saved/closed window supplies the pane
session restored on restart; restoring every open window is not implemented.
Running multiple separate application processes against one configuration is
not coordinated by this in-process window runtime.

## 12. muCommander import

File → Import muCommander settings… previews an upstream configuration directory.
It reads `bookmarks.xml`, `action_keymap.xml` (plus legacy keymap aliases),
`commands.xml` and `preferences.xml`. Choose bookmarks, shortcuts, commands and/or
supported preferences, preview, then apply. Applying writes a private backup of
the current settings and atomically saves the merged result. Intervening settings
changes are refused. Source XML is never modified.

Existing bookmark names/locations, command aliases and customized shortcut
bindings take precedence. Untouched default shortcuts can adopt supported
upstream primary and alternate keystrokes. Supported preferences include toolbar,
command/status bars, hidden files, folders-first and font size. Unsupported
properties, remote bookmarks, action mappings and command syntax appear in the
preview; credentials are not imported. XML parsing is bounded and rejects DTDs
and external entities.

Imported system commands appear in File → Imported commands. Their expanded
program/arguments are displayed for explicit confirmation before execution.
Supported placeholders are `$f` (separate argument, all selected files), `$p`
(first full path), `$n` (filename), `$b` (stem) and `$d` (current folder).
Arguments are passed directly without a shell. Pipelines, shell expressions and
unsupported placeholders are rejected during import. Review imported programs
before choosing to run them. This is a supported-subset migration, not a complete
upstream configuration converter.
