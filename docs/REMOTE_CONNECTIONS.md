# Remote connections

[← Back to README](../README.md)

Open **File → Connections** to add or edit a connection profile. Available
adapters are SMB2/3, SFTP, FTP, WebDAV, S3, HTTP, Dropbox, Google Drive, OneDrive, GCS,
and WebHDFS. Configure the endpoint, root, and service-specific fields; unused
fields may remain blank.

Remote locations use `remote://CONNECTION_ID/path`.

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

- **SFTP** uses the system OpenSSH executable, an agent or private key, and
  strict known-host verification on **macOS and Linux**. The current OpenDAL
  transport depends on the Unix-only `openssh` crate, so SFTP is unavailable in
  Windows builds and is omitted from their service picker. Existing SFTP profiles
  remain readable and report this limitation when used on Windows. Windows
  defaults to WebDAV; other remote adapters remain available. Password-based
  SFTP is not implemented.
- **Cloud adapters** accept access tokens. Google Drive, Dropbox and OneDrive also
  offer browser sign-in with PKCE, token refresh and reconnection. Register your
  own desktop client first; see [OAuth setup](OAUTH.md).
- **GCS** expects a base64-encoded service account credential payload in the
  token field.
- **HTTP** supports direct file paths rather than listing folders.

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

Remote behavior is tested through an isolated filesystem adapter and a real
loopback HTTP WebDAV fixture. Those tests establish only their tested contracts.
A disposable Samba server additionally validates direct SMB listing, upload/download,
rename, search, guarded editing and deletion. Live FTP, SFTP, S3, and cloud services
have not been tested. See the [0.5 guide](FEATURES_0.5.md) for remote editing and
search limits.

See the [validation record](../parity/VALIDATION.md) and
[upstream parity checklist](../parity/PARITY.md) for current evidence and gaps.

## Checkpointed copies and synchronization

Copies can opt into durable batch checkpoints for new targets. Verified downloads
use ranged readers; uploads append to hidden staging files when the service
supports it and restart that staged file otherwise. Interrupted final publishes
can require review of a partial destination. Synchronization accepts local/remote
or remote/remote pairs with content-based previews and stale-plan guards.
See the [next twelve-feature guide](FEATURES_NEXT.md) for the review steps,
checkpoint storage/cleanup and remaining provider limits. These workflows have
filesystem-adapter regression coverage; live-provider behavior still needs testing.
