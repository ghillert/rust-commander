# Remote connections

[← Back to README](../README.md)

Open **File → Connections** to add or edit a connection profile. Available
adapters are SFTP, FTP, WebDAV, S3, HTTP, Dropbox, Google Drive, OneDrive, GCS,
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

- **SFTP** uses the system OpenSSH executable, an agent or private key, and
  strict known-host verification. Password-based SFTP is not implemented.
- **Cloud adapters** accept access tokens. Google Drive, Dropbox and OneDrive also
  offer browser sign-in with PKCE, token refresh and reconnection. Register your
  own desktop client first; see [OAuth setup](OAUTH.md).
- **GCS** expects a base64-encoded service account credential payload in the
  token field.
- **HTTP** supports direct file paths rather than listing folders.

## Transfer behavior and limits

Remote transfers preview top-level conflicts and offer Stop, Skip, Keep both and
Replace, with per-item overrides. Replace streams the old target into a hidden
recovery copy before removal; completed copies and backups can remain after an
error. Folder conflicts apply to whole folders; remote folder merging is not
implemented. Some services lack conditional-create
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
Live FTP, SFTP, S3, and cloud services have not been tested.

See the [validation record](../parity/VALIDATION.md) and
[upstream parity checklist](../parity/PARITY.md) for current evidence and gaps.
