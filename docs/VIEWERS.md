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
Large local text uses the existing read-only pages. Hex initially displays at
most 512 KiB. Images/PDFs retain their 32 MiB input limits. Remote/archive hex
inputs are bounded to 32 MiB.

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
