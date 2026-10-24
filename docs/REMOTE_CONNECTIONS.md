# Remote connections

[← Back to README](../README.md)

Open **File → Connections** to add or edit a connection profile. Available
adapters are SMB2/3, SFTP, FTP, WebDAV, S3, HTTP, Dropbox, Google Drive, OneDrive, GCS,
and WebHDFS. Configure the endpoint, root, and service-specific fields; unused
fields may remain blank.

## Paste a path or URI

The location bar is an editable text field by default. Press **Cmd+L** on macOS
or **Ctrl+L** on Windows/Linux, paste an address, and press **Enter**. The button
beside it switches to clickable breadcrumbs; choose either default in
**Settings → Desktop appearance → Location bar**. Escape restores the current
address and cancels an outstanding URI connection. Failed input remains editable.
The preference also travels with saved workspaces.

| Address | Meaning |
| --- | --- |
| `https://sftp.kew.org/pub/data-repositories/WCVP/` | Read-only HTTPS directory index |
| `http://host/directory/` | Read-only HTTP directory index |
| `https://host/download.zip` | A direct download, displayed as one file |
| `sftp://user@host:2222/path/` | SSH file transfer, on all desktop platforms |
| `ftp://host/pub/` | FTP; empty username uses anonymous login |
| `ftps://user@host/path/` | Implicit TLS, default port 990 |
| `ftpes://user@host/path/` | Explicit AUTH TLS, default port 21 |
| `file:///Users/me/Documents/` | Local folder; local files select in their parent |
| `file:///C:/Users/me/Documents/` | Windows local folder |
| `file://server/share/folder/` | Windows UNC folder; use SMB on other platforms |
| `smb://user@nas/share/folder/` | Direct SMB share |
| `s3://bucket/prefix/` | S3 or a matching S3-compatible saved connection |
| `gs://bucket/prefix/` | Google Cloud Storage |

The actual scheme determines the protocol: the word `sftp` in the Kew hostname
does not turn its HTTPS URL into an SFTP connection. Spaces and Unicode names
are percent-encoded in displayed addresses. Ordinary paths, relative paths,
archive navigation and older `remote://CONNECTION_ID/path` locations still work.
`data:` URIs and unrecognized schemes display an explanatory error.

### Credentials and saving connections

Public locations open immediately. Missing credentials, SMB share names or cloud
configuration open a prefilled connection form. Matching saved profiles are
reused, including their configured cloud endpoints; multiple matching profiles
produce a chooser. S3-compatible endpoints and regions belong in the connection
settings, not in the bucket portion of an `s3://` address.

Browsing does not add a named sidebar connection. Use **Save connection…** beside
the location bar to name and retain one explicitly; opening it returns to the
folder you saved. Non-secret descriptors for
unnamed locations are saved so tabs, history, bookmarks and workspaces can be
restored, but they remain outside the Connections sidebar. Saving a connection
and remembering its credentials are separate choices.

Passwords are removed from displayed addresses. HTTP query strings, which can
contain signed download credentials, remain only in session memory: they are
excluded from settings, history, logs and credential-store serialization. After
restarting, paste the complete query-bearing URI again. Credentials never appear
in a saved connection descriptor.

## Credentials

Profiles contain no passwords or tokens. Secrets remain in memory unless
**Remember credentials** is selected, which uses the operating system's
credential store:

| Platform | Credential store |
| --- | --- |
| macOS | Keychain |
| Windows | Credential Manager |
| Linux | Secret Service |

There is no plaintext fallback. Credential-store UI behavior still needs live
validation on the supported platforms.

## Service-specific behavior

- **SMB2/3** connects directly without a mounted share. Set Endpoint to
  `smb://host:445`, Share to its name, and Root folder to a path within it.
  Domain usernames can use `DOMAIN\user`. The pure Rust adapter streams files
  and supports the same search/edit/transfer workflows as other writable services.

- **SFTP** supports Windows, macOS and Linux. Existing Unix key/agent profiles
  retain the system OpenSSH transport and its SSH configuration. Windows and
  password connections use the Rust SSH/SFTP transport, supporting passwords,
  private keys and the Windows OpenSSH agent. Host keys must already be trusted
  in `~/.ssh/known_hosts` (or the optional configured known-hosts file). Unknown
  and changed keys are rejected; there is no automatic trust or verification
  bypass. The Rust transport does not interpret OpenSSH host aliases or host
  certificates. Use the real hostname and a pinned public host key there.
- **FTP/FTPS** use binary, passive transfers. Both TLS styles are selectable under
  **FTP security**, including custom ports. Existing saved `ftps://` profiles
  retain their original explicit-TLS interpretation; newly pasted `ftps://` URIs
  mean implicit TLS. Certificates and hostnames are verified. A private server's
  CA can be supplied as a DER certificate using the optional TLS CA field.
  Connecting validates the root and never creates missing remote directories.
- **Cloud adapters** accept access tokens. Google Drive, Dropbox and OneDrive also
  offer browser sign-in with PKCE, token refresh and reconnection. Register your
  own desktop client first; see [OAuth setup](OAUTH.md).
- **GCS** expects a base64-encoded service account credential payload in the
  token field.
- **HTTP/HTTPS** browse recognizable Apache, nginx and Caddy HTML directory
  indexes and display direct-file URLs as a single downloadable entry. This is
  read-only: writing, moving, renaming and deleting HTTP resources are disabled.
  Ordinary websites are not filesystem listings; use **Open in browser** from
  the resulting connection dialog. HTML and JavaScript are never executed.
  Only immediate same-origin child links are listed; parent navigation uses the
  location bar and breadcrumbs. Directory indexes are limited to 8 MiB and
  20,000 cached links. Transfers remain streamed. Sizes and modification times
  are read from Apache/nginx preformatted indexes and Apache/Caddy table rows,
  including Caddy's exact byte counts and timestamp attributes. Human-readable
  listing sizes may be rounded. Index timestamps without a timezone are
  interpreted as UTC and displayed in your local timezone. Sizes/dates that the
  server omits display as `—`. File operations fetch authoritative metadata and
  fingerprint content when HTTP offers no usable version information.

## Transfer behavior and limits

Remote transfers preview conflicts and offer Stop, Skip, Keep both and
Replace, with per-item overrides. Replace streams the old target into a hidden
recovery copy before removal; completed copies and backups can remain after an
error. Folder conflicts can be reviewed recursively with Merge existing folders enabled;
per-file choices and recovery copies apply to nested conflicts. Some services lack conditional-create
support: conflict checks do not lock out simultaneous writes by other clients.
Failed remote transfers can leave partial output.

Remote deletion is permanent and requires confirmation. Remote rename
streams a copy, checks source metadata and removes the original after success.
Cancellation can keep both copies. Nested remote archives open read-only in a
bounded temporary cache; remote archive editing is not implemented. For local transfer and
recovery behavior, see [File operations](FILE_OPERATIONS.md).

## Validation status

Automated tests cover filesystem adapters, loopback HTTP directory/download
fixtures, and real disposable OpenSSH, FTP, WebDAV and Samba servers. The URI
transport harness additionally exercises anonymous FTP, implicit and explicit
FTPS, Rust SFTP password/key authentication, strict certificate/host-key rejection,
range reads, guarded saves, Unicode paths, cancellation/resume, conflicts,
synchronization, rename and deletion. The portable harness is configured to run in native
Windows, macOS and Linux CI jobs; Linux also checks the Rust transport against
OpenSSH. No live private cloud accounts are used.

To run the portable fixtures locally:

```sh
python3 -m venv .tools/uri-tests
.tools/uri-tests/bin/python -m pip install -r scripts/uri-fixtures/requirements.txt
.tools/uri-tests/bin/python scripts/test-uri-transports.py
```

On Windows, use `.tools/uri-tests/Scripts/python.exe` for those two commands.
The fixtures bind only to loopback, generate disposable credentials/certificates,
write logs under `target/uri-transport-logs`, and clean up their temporary roots.
They never modify user SSH files, stored credentials or application settings.
The Kew listing has a separate opt-in, read-only acceptance test:

```sh
cargo test --test uri_navigation kew_https_directory_smoke -- --ignored --exact
```

See the [validation record](../parity/VALIDATION.md) and
[upstream parity checklist](../parity/PARITY.md) for current evidence and gaps.

## Checkpointed copies and synchronization

Copies can opt into durable batch checkpoints with merges and fixed conflict decisions,
including replacement recovery copies. Pause/resume, a live bandwidth limit and
bounded automatic retries are available in the transfer controls. Verified downloads
use ranged readers; uploads append to hidden staging files when the service
supports it and restart that staged file otherwise. Interrupted final publishes
can require review of a partial destination. Synchronization accepts local/remote
or remote/remote pairs with content-based previews and stale-plan guards.
See the [next twelve-feature guide](FEATURES_NEXT.md) for the review steps,
checkpoint storage/cleanup and remaining provider limits. These workflows have
filesystem-adapter regression coverage; live-provider behavior still needs testing.
