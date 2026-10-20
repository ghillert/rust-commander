# Releasing Rust Commander

Rust Commander uses GitHub Actions to build and test native applications,
cargo-packager **0.11.8** to create app bundles and desktop packages,
dmgbuild **1.6.7** to lay out the Mac DMG without Finder automation, and
JReleaser **1.26.0** to assemble a **draft GitHub release**. Publishing is a separate, manual action.
The entire release runs on GitHub-hosted runners; your Mac need not stay online.

## First-time repository setup

1. Merge the release workflow and its scripts into `main`. A manually dispatched
   workflow must exist on the default branch before GitHub shows its Run button.
2. In **Settings → Actions → General**, allow the actions used by the repository.
   Under **Workflow permissions**, enable **Allow GitHub Actions to create and
   approve pull requests**. The workflow itself grants `contents: write` only to
   preparation/publication and `pull-requests: write` only to publication.
3. Ensure repository rules permit Actions to create `release/v*` branches and
   `v*` tags. No bypass of `main` protection is required: version changes return
   through a PR. GitHub must be able to sign and verify the preparation commit.
4. Ensure enough Actions minutes/storage are available for four native build
   jobs. The platform jobs can take considerably longer than incremental local
   builds, especially on a cold cache.
5. The publication job installs Temurin Java 17 and the SHA-256-verified
   JReleaser **1.26.0 Java distribution**. No local Java installation is needed.
6. No Apple credentials, personal GPG key, Windows certificate, package-registry
   token, or JReleaser secret is needed initially. The GitHub workflow token is
   passed to `gh` and JReleaser for this repository only.

The initial distribution targets are macOS 15+ (ARM64 and Intel separately),
Windows x64, and Ubuntu 24.04+ x64. Other Linux distributions must provide a
compatible glibc/system graphics stack; AppImage does not guarantee compatibility
with every distribution. There is no Linux/Windows ARM64 package yet.

Mac apps are **ad hoc signed and not notarized**. Windows executables/installers
are **not Authenticode signed**. GitHub's verified release commit is a source-code
signature; it does not replace either platform's application signing system.

## Create a release

First merge the intended changes into `main` and check ordinary CI. Decide on a
new version, for example `0.8.0`. Use SemVer without a leading `v` or `+build`
metadata. Prereleases such as `0.8.0-rc.1` are supported. Numeric components must
fit Windows version fields (0–65535); numeric prerelease components cannot have
leading zeroes. Choose a version different from the current `Cargo.toml` version
so the synchronization PR contains an actual version change.

### GitHub website

1. Open **Actions → Release → Run workflow**.
2. Select **main**, enter the desired version, and click **Run workflow**.
3. Follow preparation, the four platform jobs, and publication in the run view.
4. Open the draft release and version PR linked in the final job summary.

### GitHub CLI

```sh
gh workflow run release.yml --ref main -f version=0.8.0
gh run list --workflow release.yml --limit 5
gh run watch RUN_ID
```

The workflow captures the selected `main` commit at dispatch. Later changes to
`main` cannot enter that release. Release runs are serialized. Avoid queuing
multiple releases at once; GitHub concurrency retains only a limited pending
queue.

## What the workflow creates

- A `release/v0.8.0` branch and GitHub-signed, verified preparation commit with
  subject `chore: prepare release`. Only the root version in `Cargo.toml` and
  `Cargo.lock` changes; dependencies are not upgraded. The commit records the
  source SHA and original workflow run ID for safe retries.
- Builds and tests from that exact commit, using the same checks as ordinary CI.
- A `v0.8.0` tag at the tested commit, created after all packages pass. This tag
  is not moved by retries.
- A draft release titled **Rust Commander 0.8.0**, containing these six assets:

| Platform | Filename |
| --- | --- |
| Mac, Apple Silicon | `rust-commander-0.8.0-macos-arm64.dmg` |
| Mac, Intel | `rust-commander-0.8.0-macos-x64.dmg` |
| Windows installer | `rust-commander-0.8.0-windows-x64-setup.exe` |
| Windows portable | `rust-commander-0.8.0-windows-x64.zip` |
| Linux AppImage | `rust-commander-0.8.0-linux-x64.AppImage` |
| Ubuntu/Debian package | `rust-commander-0.8.0-linux-x64.deb` |

The release also contains `checksums_sha256.txt` and `release-manifest.json`.
The manifest records the source commit, hashes, sizes, and signing status.
JReleaser generates the changelog and checksums. GitHub also supplies source
archives for the tag. All downloads stay attached to that version's release
page after publication; CI artifact retention does not remove release assets.

A PR synchronizes the version change back into `main`. Draft creation does not
wait for the PR to merge. If `main` changes meanwhile, resolve the version PR
normally. Do not amend the tested release commit or move the release tag.

## Review and publish

1. Read the draft notes, correct the user-facing summary, and check the signing
   and platform limitations. For a first release, inspect the generated history
   carefully because there may be no previous tag to compare against.
2. Download the packages and checksum file from the draft. Maintainers can use:

   ```sh
   gh release download v0.8.0 --dir release-review
   cd release-review
   shasum -a 256 -c checksums_sha256.txt   # macOS
   # sha256sum -c checksums_sha256.txt    # Linux
   ```

   On Windows, use `Get-FileHash FILE -Algorithm SHA256` and compare with the
   corresponding entry in the checksum file. Download every listed file before
   using the whole-file verification command.
3. Check the manifest's commit against the release tag. Install and smoke-test
   packages on available machines, including navigation, archive access, and
   copying disposable files. Review the native-runner installation test results.
4. Merge the version synchronization PR so the next release starts from the
   correct version. If the PR is blocked, resolve that independently; the draft
   packages already refer to their tested commit.
   A PR created with `GITHUB_TOKEN` does not automatically trigger ordinary PR
   workflows. If branch protection requires the CI checks, use **Actions → CI →
   Run workflow**, selecting `release/v0.8.0`, or run
   `gh workflow run ci.yml --ref release/v0.8.0` before merging.
5. Click **Publish release** in GitHub. Only then are the draft assets publicly
   downloadable. For prereleases, keep **Set as a pre-release** selected.

Promote an RC by running a new release with the stable version, for example
`0.8.0-rc.1` → `0.8.0`. This rebuilds correct stable-version metadata. Do not
rename RC files or retarget its tag.

Executables, filenames, and displayed versions retain the full prerelease version.
macOS bundle and Windows numeric version fields use its numeric core. Debian
metadata uses `0.8.0~rc.1`, so APT correctly upgrades an RC to `0.8.0`.

## Install the downloads

### macOS

Choose **ARM64** for Apple Silicon (M-series) or **x64** for Intel. Mount the DMG,
drag **Rust Commander.app** onto **Applications**, eject the DMG, and launch the
installed application. Required non-system libraries are included in the app;
Homebrew is not required.

The DMG Finder window displays these installation and first-launch steps on
its background. The editable source is `assets/dmg/install-background.svg`;
pre-rendered 1× and 2× images are combined into a HiDPI background. The release
smoke test checks the Finder layout and background in both Mac packages.

Because these initial builds are unnotarized, macOS may block the first launch.
After attempting to open it, use **System Settings → Privacy & Security → Open
Anyway** for Rust Commander, then confirm. The option appears for about an hour
after the blocked launch; try opening the app again if it has disappeared.
Only do this for a download you trust and whose checksum you verified. Do not
disable Gatekeeper globally. See
[Apple's instructions for apps from unidentified developers](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac).

### Windows

Run the `-setup.exe` installer for a per-user installation, or extract the ZIP
and run `rust-commander.exe`. Keep the accompanying DLLs alongside the executable.
The installer supplies shortcuts and uninstall support; uninstall preserves app
settings. Windows may show an unknown-publisher/SmartScreen warning because the
downloads are not Authenticode signed.

### Linux

For Ubuntu 24.04 or later, install the DEB with APT so dependencies are resolved:

```sh
sudo apt install ./rust-commander-0.8.0-linux-x64.deb
```

For AppImage:

```sh
chmod +x rust-commander-0.8.0-linux-x64.AppImage
./rust-commander-0.8.0-linux-x64.AppImage
# If FUSE mounting is unavailable:
./rust-commander-0.8.0-linux-x64.AppImage --appimage-extract-and-run
```

The system still supplies graphics drivers and desktop services. SFTP needs an
OpenSSH client (`ssh`), and external-application opening needs `xdg-open`.
Opening or emptying Trash needs `gio` and a GVfs Trash backend; on Ubuntu these
are supplied by `libglib2.0-bin` and `gvfs-backends`. The DEB declares these
dependencies; AppImage users should install them through their distribution.
Configure a desktop Secret Service if you
want to save credentials in a system keyring. Windows currently has no SFTP
adapter; the other documented remote adapters remain available.

## Failed builds, retries, and duplicate versions

- **Preparation fails:** inspect its log for version syntax, permissions, branch
  ownership, a conflicting tag, or an unverified commit. Fix repository settings
  and rerun the original run if no source change is necessary.
- **A build/package test fails:** download its logs and packaging diagnostics.
  The diagnostics artifact includes any packages produced before the failure;
  these have **not** passed installation tests and must not be published.
  A transient runner/network problem can be retried using **Re-run failed jobs**
  or `gh run rerun RUN_ID --failed`. A source fix requires a new release version
  from updated `main`; the old run deliberately keeps its original source.
- **Publication is interrupted:** rerun **failed jobs in the original run**.
  Existing uploads must match the local hashes and sizes. Completed assets and
  edited draft notes are preserved. An empty GitHub upload starter may be
  removed and retried. A different existing file is never overwritten.
- **Publication tooling needs a source fix:** merge the fix into `main`, then
  run **Actions → Resume release publication → Run workflow** on `main`. Enter
  the original version and original **Release** workflow run ID. This verifies
  that all four native package jobs succeeded, checks the original signed
  release branch, downloads those exact artifacts, and resumes the draft and
  version PR without rebuilding or moving the release commit. Equivalent CLI:

  ```sh
  gh workflow run release-resume.yml --ref main \
    -f version=0.8.0-rc.1 -f original_run_id=RUN_ID
  ```

  Use this only while all four original artifacts are retained. The recovery
  job has the same published-release and mismatched-asset protections as the
  normal publisher.
- **Do not rerun successful build jobs after a partial upload.** Packages such
  as DMGs contain generated metadata and need not be byte-identical after a
  rebuild. Reuse the original successful build artifacts, retained for 14 days.
  If they have expired, start a new version rather than replacing uploaded bytes.
- **A new dispatch uses an existing version:** preparation rejects a branch
  owned by a different original run. Resume the original run, or use a new
  version. Published/immutable releases and conflicting tags are always refused.
- **The version PR fails:** the complete draft may already exist. Enable Actions
  PR creation or resolve repository rules, then rerun the publication job.
- **An abandoned draft:** leave it unpublished. If you deliberately delete an
  unpublished draft/tag/branch, review all three together; this is manual
  maintenance, never an automatic retry strategy. Published tags must stay fixed.

Use the Actions job logs first. Publication diagnostics include JReleaser's
`trace.log` and `output.properties`; build jobs retain packaging diagnostics on
failure. The final job summary is only written after the complete release asset
set has been verified and the synchronization PR is available.

## Local development and rehearsal

Python 3.11+ is required by the release scripts. Install the pinned packager with:

```sh
./scripts/cargo-local.sh install cargo-packager --version 0.11.8 --locked --root .tools/packager
python3 scripts/release/tests/test_release.py
./scripts/package-macos.sh
python3 -m venv .tools/release-python
.tools/release-python/bin/python -m pip install --require-hashes --only-binary=:all: -r scripts/release/requirements-macos.txt
.tools/release-python/bin/python scripts/release/package.py --target macos-arm64
.tools/release-python/bin/python scripts/release/smoke.py --target macos-arm64
```

Use `macos-x64` on an Intel Mac. The app packaging script sets the macOS 15
deployment target and required linker padding. Package output is under
`dist/release/`; intermediate bundles are under `target/release-packaging/`.
These directories are ignored by Git. Icon sources and a standard-library-only
regeneration script are included under `assets/icons/` and `scripts/release/`.
To change DMG artwork, edit `assets/dmg/install-background.svg`, install
`CairoSVG==2.8.2` in a temporary virtual environment, and run
`scripts/release/render-dmg-background.py` with that environment's Python.

The offline tests exercise preparation, version/lockfile consistency, conflicting
tags, signature checks, asset tampering, and interrupted-upload recovery. They
do not contact GitHub or create releases. For the first live rehearsal, dispatch
an explicitly chosen prerelease after this workflow is merged, follow this guide,
and leave the resulting release as a draft until its downloads pass review.
Local macOS tests do not establish Windows/Linux/Intel compatibility; inspect
their native CI jobs before publishing.

## Adding Apple signing and notarization later

Obtain a **Developer ID Application** certificate through an eligible Apple
Developer Program membership. Export it with its private key as a password-
protected `.p12`. Obtain notarization credentials, preferably an App Store Connect
API key. Configure these as GitHub environment/repository secrets, not files in
the repository:

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE_P12_BASE64` | Exported Developer ID Application certificate and private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting that export |
| `APPLE_API_PRIVATE_KEY` | App Store Connect `.p8` key contents |
| `APPLE_API_KEY_ID` | Key ID |
| `APPLE_API_ISSUER_ID` | Issuer ID |

These names document the future integration; adding secrets alone does **not**
enable notarization in the current workflow. Add an explicit notarized packaging
mode that imports the certificate into a temporary runner keychain, signs bundled
libraries and the app from the inside out with hardened runtime and a secure
timestamp, submits with `xcrun notarytool --wait`, staples tickets, and verifies
the final DMG. Missing credentials or rejected notarization must fail that mode,
never silently fall back to an unnotarized public download. Clean up the temporary
keychain and key files even on failure. Update release notes and the manifest to
report the actual signing status.

References: [GitHub certificate setup](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications),
[Apple Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## Release checklist

- [ ] Intended changes merged; ordinary CI passes.
- [ ] New version selected; release dispatched from `main`.
- [ ] Four native build/package jobs pass.
- [ ] Six packages, manifest, and checksum file present on the draft.
- [ ] Checksums, source commit, version metadata, and installation tests reviewed.
- [ ] Release notes and signing/platform limitations checked.
- [ ] Version synchronization PR merged.
- [ ] Stable/prerelease designation correct; draft published manually.
