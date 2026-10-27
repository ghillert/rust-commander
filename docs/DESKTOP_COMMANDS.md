# Desktop commands

[← Back to README](../README.md)

## Bring all windows forward

**Window → Bring all windows forward** restores minimized windows and asks the
desktop to raise every open Rust Commander window in this process. The window
that invoked the command is raised last and retains focus. Pane contents, marks,
tabs and window geometry are unchanged. Window managers/compositors can restrict
focus requests; separate application processes are not included.

## Explore bookmarks

**Navigate → Explore bookmarks…** opens a dedicated bookmark browser. Filter the
list, select a bookmark, and open it in the current or opposite pane. Normal tab
lock/navigation rules still apply.

The same dialog can rename or remove a bookmark, move it up/down, or add the
current location under the entered name. Removing a bookmark does not remove its
files. Names must be unique without regard to case. A change in another window
requires refreshing before editing; a failed settings save rolls back the edit.
Filtering clears the selection so a hidden bookmark cannot be edited accidentally.
Reordering is disabled while filtering. Local, archive and saved remote locations
retain their actual location identity rather than a reparsed display URL.

## Email files

Mark files/folders and choose **File → Email files…** or the file context menu.
The selection is frozen. A worker prepares local, remote or archive attachments;
directories are flattened into regular files. Click attachment rows to include
or exclude them, then enter recipients, subject and body. Review the complete
attachment list before choosing **Send email**.

**SMTP settings…** holds the hostname, port, sender and optional username. Choose
TLS on connection (normally port 465) or required STARTTLS (normally 587); changing
the security mode updates these standard ports while preserving a custom port.
TLS certificates must validate. There is no unencrypted or opportunistic-TLS
fallback. Enter the SMTP password for each send when a username is configured.
The password stays in memory. Only the non-secret SMTP settings are saved;
recipients, subject, body, attachment data and passwords are not saved in settings,
history or the operation queue.

Preparation accepts at most 100 regular files and 25 MiB of raw attachment data,
with traversal bounded to 5,000 entries/64 levels. Links/special local files are
refused. Archive entry validation and provider read limits still apply. Prepared
bytes are checked while reading and retained for review; subsequent source changes
do not silently change those bytes. MIME encoding increases the final message
size, and a server may enforce a smaller limit. Folder path separators become
` - ` in attachment filenames; original files are unchanged.

Sending runs on a worker with a 30-second SMTP I/O timeout. Controls and window/
application closing are protected while waiting for its result. A successful
result means the SMTP server accepted the message, not that the recipient has
received it. An interrupted or failed submission can be ambiguous: check the
server/mailbox before retrying. Messages are never queued, resumed after restart
or retried automatically. Closing the dialog discards an unsent draft.

The mail service must permit SMTP submission and the selected authentication
method; an app password may be needed. OAuth-only mail accounts are not supported
by this SMTP form. No live account is contacted by the regression tests.

## Documentation, community, website and donations

The **Help** menu includes:

- **Documentation:** the repository README and its linked guides.
- **Community support:** the repository Issues page. The project has no separate
  forum enabled; the upstream `GoToForums` command maps to this support destination.
- **Project website:** the Rust Commander repository.
- **Donate…:** a reviewable HTTPS donation link, defaulting to
  [GitHub Sponsors for ghillert](https://github.com/sponsors/ghillert), as declared
  by the repository's published `.github/FUNDING.yml`. An edited link is saved
  before opening it. Credentials and non-HTTPS links are rejected. Opening the
  page does not make a payment.

These actions open the system browser and report launch errors. The commands are
also available to configurable shortcuts. Existing Help, update checks,
diagnostics and reviewed bug reports remain in the same menu.

## Debug console

**Help → Debug console…** shows live application events, including command
dispatch, operation start/result summaries, bookmark changes and SMTP outcomes.
The buffer is shared by windows in this process, holds at most 2,000 events and
is not written to disk. Events have a UTC time, sequence number and category.

Filter the displayed events, pause/resume the display, copy the filtered snapshot
or clear the buffer. Pausing the display does not stop event collection. Paths,
credentials, editor/email contents and attachment bytes are excluded. Detailed
operation failures remain in the normal status/history workflow. This is an
application event console, not a shell or a capture of arbitrary library/stdout
output. Review any additional logs before attaching them to a bug report.

See [validation](../parity/VALIDATION.md) for fixture/native acceptance coverage.
