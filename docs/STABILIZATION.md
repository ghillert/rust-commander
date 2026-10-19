# Stabilization and integration testing

[← Back to README](../README.md)

This pass adds repeatable verification for the existing features and fixes faults
found by real protocol tests. It does not claim complete muCommander parity.

## Cross-platform checks

The GitHub CI matrix runs formatting, all-target/all-feature Clippy and tests on
macOS, Windows and Ubuntu. Each platform also runs optimized transfer/checkpoint
regressions and builds a release package. The Windows-only stabilization test
copies a protected DACL and verifies that inheritance remains disabled.

Ubuntu additionally runs the real-service harness, a native clipboard round-trip,
the GTK drag-source test and file/PDF snapshots under Xvfb with both software and
femtovg renderers. OpenGL uses Mesa software rendering in that environment. The
workflow retains service logs and snapshots, including when a later check fails.
These checks complement interactive desktop testing; they do not certify every
native dialog, screen reader, display server or desktop environment.

## Disposable real remote servers

On macOS or Linux, with Docker and OpenSSH available:

```sh
python3 scripts/test-remote.py
```

For a focused rerun:

```sh
python3 scripts/test-remote.py --service webdav
```

The harness builds an official Debian image and starts OpenSSH/SFTP, pyftpdlib
FTP, WsgiDAV and Samba. Every published port binds to `127.0.0.1`. Storage stays
inside a new disposable container; no host directories are mounted. It uses public
test credentials, a temporary client key and a private known-hosts file pinned to
the created container's host key. It leaves the user's SSH configuration, keychain
and Rust Commander profiles untouched. An SSH wrapper applies that private
configuration only to the harness child process.

The four protocol tests cover Unicode/spaces, listing, upload/download, ranged
reads, recursive conflict merges, guarded save refusal, cancellation and durable
retry, checkpoint publication, existing-target refusal, local/remote synchronization,
stale preview refusal, remote-to-remote copy, rename and recursive deletion.
Rejected password authentication is checked for FTP, WebDAV and SMB; SFTP uses
key authentication with strict host-key verification.

The runner removes its container and temporary keys/configuration on normal exit,
test failure, Ctrl-C and SIGTERM. A 15-minute test watchdog bounds stalled runs.
SIGKILL or a machine crash can bypass cleanup: inspect containers carrying the
`io.github.ghillert.rust-commander.fixture=true` label before removing an orphan.
The reusable fixture image remains in Docker's image cache. Reports go to
`target/remote-service-logs/`; they contain only disposable fixture details.

This validates the shared workflows against those implementations. It does not
validate cloud account registration/OAuth, S3/GCS, other server implementations,
TLS certificate deployments or all provider-specific concurrency behavior.

## Interruption and recovery

```sh
./scripts/cargo-local.sh test --locked --test stabilization
./scripts/cargo-local.sh test --release --locked --test stabilization --test features_08
```

A parent test launches a worker subprocess and makes it exit during a streamed
copy without running destructors. A fresh process/session verifies and resumes
that checkpoint, compares all source/destination bytes and checks cleanup.
Another regression interrupts replacement during a move and verifies that the
old target and complete source survive with uncommitted staging removed.

The helper `checkpoint_crash_child` is deliberately ignored in normal runs; the
parent invokes it with its own temporary fixture. Do not run every ignored test
indiscriminately: other ignored cases require a live server or desktop.

## Keyboard and assistive technology

Tab and Shift-Tab switch panes while a file list owns focus. Elsewhere they keep
normal control traversal. Toolbar buttons and Places entries participate in Tab
navigation; Places entries activate with Enter/Space. The tree accepts keyboard
focus and its existing arrow/Enter navigation. File lists expose item selection,
counts and accessible activation. Tree rows expose selected/expanded states and
accessible open/expand actions. Focus-next shortcuts start at the actual focused
pane control, and modal traversal keeps background panes unavailable.

Debug builds include Slint element metadata so tests inspect the actual accessible
tree and invoke its actions. Release builds omit that debug metadata. These
regressions cover semantics and routing; full VoiceOver/NVDA/Orca interaction
remains a separate native validation task.

## Two-renderer smoke test

With a running desktop, or under Xvfb on Linux:

```sh
./scripts/cargo-local.sh build --locked --features screenshot
python3 scripts/desktop-smoke.py
```

This creates isolated settings and fixture files, renders the file view and a
rotated PDF with both renderers, verifies valid image dimensions and exits each
app process. It records PNGs and logs in `target/desktop-smoke/`. This is a rendering
smoke check; it is not a pixel-perfect assertion of visual appearance.

## Fixes found during this pass

- Recursive remote deletion now visits children before parents, bounds traversal
  and verifies that each reported deletion actually removed the item. The live
  FTP fixture exposed parents left behind when a bulk delete treated a nonempty
  directory response as success.
- Providers that require a single upload buffer now use a private temporary file
  and a read-only memory map. This avoids the repeated-write failure found with
  WebDAV uploads larger than the storage adapter's buffer, and avoids collecting
  the entire file in a heap buffer. Staging remains cancellable and needs temporary
  disk space. Provider commit calls remain subject to their I/O timeouts and
  cooperative cancellation limits.

The tree also resolves the active local location on its worker, so aliases such
as macOS `/var` correctly reveal the corresponding physical folder. This also keeps hidden ancestors of that active folder visible. It does not enable
recursive expansion through symlink descendants.
