# Open As and PDF text tools

[← File operations](FILE_OPERATIONS.md#viewers-and-text-editing)

## Open As

Select one file and choose **File → Open As…**, or use its right-click menu.
Choose Text, Hexadecimal, Image, PDF, ZIP, 7z, TAR, a compressed TAR format, or
Other archive (detect format). This overrides the filename's extension for this
opening. The source's name and contents stay unchanged. Open With continues to
choose an external application.

The choice works for local, remote and archive entries. Text uses BOM-aware
decoding with a Windows-1252 fallback; binary input containing NUL is rejected.
Built-in editing accepts up to 16 MiB. Larger local and remote text uses read-only
256 KiB pages; archive members use a private snapshot of at most 2 GiB. Text and
hex search/jump work on these sources. Hex displays bounded 64 KiB windows.
Remote paging requires known content length and uses available metadata tokens
to reject changed sources; providers without ETag/mtime have weaker change detection.
Images/PDFs accept up to 512 MiB through immutable file-backed snapshots, retaining
decoded image and page-cache limits. Archive paging decompresses a member once;
subsequent pages use its private snapshot. Disk space and initial decompression
cost still apply.

Archive overrides use a private read-only copy with the chosen suffix, checked
before opening. The source parent is retained for parent navigation. The existing
cache limits apply: 128 MiB per file, eight nesting levels and 512 MiB in total.
Close the application to clear the cache. These interpreted copies cannot be
edited through Archive tools. Choose explicit ZIP/7z and enter the optional archive
password for password-protected files. Other archive uses libarchive's format
detection and encryption support.

## PDF search and selection

- Drag over text on the rendered page to select it. Blue highlights follow
  zoom, fit-width and clockwise rotation. **Copy selection** or **Mod+C** copies
  the selected Unicode text; **Mod+A** selects the current page's text.
- **Text view** displays the current page in a read-only, selectable text field.
  Use normal mouse/keyboard selection and copying there. **Page view** returns
  to the rendered page.
- **Mod+F** focuses **Find PDF text**. Enter a literal search and use **Find next**
  or **Find previous**. Searches ignore case, cross page boundaries, wrap around
  the document, navigate to the match and highlight it. Wrapping is reported in
  the status. Matches within a page can include line breaks when those are in the
  query; phrases do not span separate PDF pages.
- Search runs on a worker, can be cancelled, and is cancelled when the viewer
  closes. Password-protected PDFs use the password entered to unlock the viewer;
  the password is held in memory only.

Extraction uses the same Hayro interpreter and page transform as rendering.
Unicode mappings, ligatures and surrogate pairs retain UTF-8 selection offsets.
Text and glyph positions are bounded to 2 MiB and 65,536 glyphs per page; document
search supports up to 10,000 pages and a 4,096-byte query. Search holds one text
page at a time; the three-page render cache also retains those pages' text.
If a page exceeds the extraction limit, it remains viewable with an explanation.

Image-only scans need OCR, which is not included. Fonts without usable Unicode
mappings cannot provide complete text. Reading order and inferred whitespace in
complex columns, vertical writing and unusual PDFs may differ from their visual
layout. Forms and annotation editing remain outside the built-in viewer.

## External editing and draft recovery

Reviewed external editing accepts up to 2 GiB for local/remote files and writable
archive members. Original and edited snapshots use private temporary files and
read-only mappings; comparisons and remote backup verification stream bounded
buffers. Save-back still checks original contents/archive identity and retains
existing recovery/conditional-write behavior. A provider without conditional
writes cannot exclude a concurrent writer atomically.

The built-in editor autosaves dirty drafts after a 500 ms debounce under the
private configuration `editor-drafts` directory. A restart offers recovery; use
**Operations → Recover editor drafts…** to reopen or discard them later. Recovery
retains the original bytes, encoding, BOM and line endings. Saving a recovered
draft refuses a changed original. Successful clean saves and explicit discards
retire the draft ID so a late worker cannot resurrect it. Updates use atomic
replacement and file synchronization; Unix draft folders/files use 0700/0600.
An interruption before the first completed autosave can lose the latest input.
External application working copies do not yet have restart recovery.
