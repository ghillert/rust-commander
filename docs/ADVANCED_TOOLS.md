# Advanced search, comparison and synchronization

[← Back to README](../README.md)

## Find files

Choose **Operations → Find files** in a local or connected remote folder. Filename
patterns match either a basename or a path relative to the search root. Multiple
patterns are separated by semicolons, for example `*.rs;*.toml` or
`src/**/*.rs;tests/**/*.rs`. Filename matching is case-insensitive unless **Match
filename case** is selected.

**Containing text** searches file contents literally by default. **Match content
case** controls Unicode case matching independently of filename matching. Select
**Content is a regular expression** for expressions such as `^error: \d{3}$`.
Regex matches one line at a time, including CRLF input. It does not match across
lines. Literal searches can match across both lines and read-buffer boundaries.
Text patterns search UTF-8 bytes; there is no automatic legacy/UTF-16 transcoding
in the recursive content search.

Literal search streams the complete file with bounded buffers; the previous local
16 MiB and remote 2 MiB file-size cutoffs no longer apply. Regex also streams files,
but limits each individual line to 8 MiB. A longer line is reported as an entry
that could not be searched, rather than silently treating it as a nonmatch. Content
patterns are limited to 16 KiB. Symbolic links and local special files are not
opened for content searching.

Additional filters:

- **Exclude files/folders:** semicolon-separated globs such as `.git;target;*.tmp`.
  An excluded directory and all its descendants are skipped.
- **Minimum/maximum file size:** inclusive byte counts; leave blank for no bound.
  Directories and entries without a known size do not match size filters.
- **Modified on/after / on/before:** inclusive RFC 3339 timestamps, for example
  `2026-10-23T00:00:00Z` or `2026-10-23T08:00:00-10:00`. Entries without a known
  modification time do not match date filters. Metadata filters use available
  directory-listing metadata, which can be approximate on HTTP indexes.

Changing a search field cancels obsolete work and discards its result. **Stop**
cancels a running search; **Open folder** navigates to a result's parent. Errors
are reported separately from results. Maximum results remains configurable from
1 to 100,000; traversal is bounded to 100,000 entries and 256 levels, with at most
1,000 individual error messages retained. Reaching a bound is reported.

## Compare folders

**Operations → Compare folders** opens a review dialog for the two pane locations.
It supports local/local, local/remote and remote/remote pairs. Include/exclude
globs use the same relative-path and basename rules as search.

**Verified file contents** remains the default and hashes both file streams.
**Quick metadata (unverified)** compares only available listing sizes and
modification times, avoiding file-content downloads. Matching metadata is explicitly
labelled **Metadata match (unverified)**; unavailable size/date information produces
**Unknown metadata (unverified)**. Neither result proves equal contents. Changing
filters or mode cancels obsolete work. Comparison is bounded to 100,000 entries
and 64 levels; the dialog displays the first 5,000 rows. Links are reported without
being followed, and archives are not comparison roots.

Use **Text diff** on a file present on both sides for an aligned, read-only view.
Line numbers and highlighted `!` rows identify insertions, removals and replacements;
the two columns share the dialog's scroll position. Decoding supports UTF-8 and
BOM-marked UTF-16. It rejects binary/invalid input and files over 1 MiB per side.
Alignment is bounded to 4,000 lines per side and about four million cells; limits
produce an error. A diff is a snapshot, never an editor or a merge action. No file
is written. Files with different byte encodings can have identical decoded text.

## Shared presets and filtered synchronization

Comparison and **Operations → Synchronize** share up to 30 named presets. Enter a
name and choose **Save preset** to add or replace its filters; comparison presets
also retain the comparison mode. **Load** applies a preset to the current folders;
**Delete** removes it. Presets persist with preferences and are shared by application
windows. They never store folder locations, connection credentials, deletion choices
or reviewed plans. Loading one in Synchronize uses only its filters.

Synchronization always checks contents, regardless of a preset's quick comparison
mode. Changing filters, roots or direction invalidates the preview and deletion
confirmation. Preview and review again before applying.

With active filters, missing folders are created separately and eligible files
are planned individually. This prevents copying excluded descendants through a
whole-folder copy. Structural folder rows can create empty folders even when no
matching child remains. Directory deletion and file/folder replacement are refused
with active filters so excluded contents cannot be removed as a side effect.
New structural folders use destination defaults; their source directory ACLs and
other directory metadata are not copied. Individual files use the normal transfer path.
Skipping a structural folder row does not skip independently selected child rows.
Individual file deletions retain the existing opt-in and confirmation requirements.

The existing stale-plan, source, destination, read-only and recovery guards still
apply. Whole-root fingerprints remain conservative: changes to excluded files can
invalidate a preview and fingerprinting can still read excluded contents. Existing
link/special-file inspection restrictions remain. Local plans retain local Undo;
plans involving a remote location retain recovery copies and do not claim local Undo.
