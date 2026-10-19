#!/usr/bin/env python3
"""Release preparation/publication. Requires Python 3.11+, gh, and JReleaser.

All writes are confined to a release branch, a new tag, and an unpublished draft.
The original Actions run ID and source SHA identify retries, not a moving branch.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib
import urllib.parse

ROOT = Path(__file__).resolve().parents[2]
TARGETS = {
    "macos-arm64": ["dmg"], "macos-x64": ["dmg"],
    "windows-x64": ["setup.exe", "zip"], "linux-x64": ["AppImage", "deb"],
}


def version_parts(version):
    match = re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?", version)
    if not match or len(version) > 100:
        raise ValueError("Use SemVer without a v prefix or build metadata, e.g. 0.8.0 or 0.8.0-rc.1")
    core = tuple(map(int, match.group(1, 2, 3)))
    if any(n > 65535 for n in core):
        raise ValueError("Version components must fit Windows' 16-bit version fields")
    pre = match.group(4)
    if pre and any(p.isdigit() and len(p) > 1 and p.startswith("0") for p in pre.split(".")):
        raise ValueError("Numeric prerelease components cannot have leading zeroes")
    return core, pre


def asset_names(version, target=None):
    version_parts(version)
    return [f"rust-commander-{version}-{t}{'-' if ext == 'setup.exe' else '.'}{ext}" for t, exts in TARGETS.items()
            if target is None or t == target for ext in exts]


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def bump_files(manifest, lock, version):
    """Change only the root package version; never resolve or update dependencies."""
    version_parts(version)
    package = tomllib.loads(manifest)["package"]
    old = package["version"]
    if old == version:
        raise ValueError("Choose a version different from Cargo.toml so the version synchronization PR has a change")
    manifest, count = re.subn(
        r'(?ms)(^\[package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*")[^"]+("[^\n]*)',
        lambda m: m[1] + version + m[2], manifest, count=1)
    if count != 1:
        raise ValueError("Cannot locate the root Cargo.toml package version")
    blocks = lock.split("[[package]]")
    replaced = 0
    for i, block in enumerate(blocks[1:], 1):
        parsed = tomllib.loads(block)
        if parsed.get("name") == package["name"] and "source" not in parsed:
            if parsed["version"] != old:
                raise ValueError("Cargo.toml and Cargo.lock disagree")
            blocks[i], count = re.subn(r'(?m)^version = "[^"]+"', f'version = "{version}"', block, count=1)
            replaced += count
    if replaced != 1:
        raise ValueError("Expected exactly one root package in Cargo.lock")
    return {"Cargo.toml": manifest, "Cargo.lock": "[[package]]".join(blocks)}


class GitHub:
    def __init__(self, repo):
        self.repo = repo
        self.prefix = f"repos/{repo}/"

    def request(self, method, path, data=None, missing=False):
        # gh handles authentication and redirects without exposing the token in argv.
        command = ["gh", "api", "--method", method, path]
        if data is not None:
            command += ["--input", "-"]
        result = subprocess.run(command, input=json.dumps(data) if data is not None else None,
                                capture_output=True, text=True)
        if result.returncode:
            if missing and "(HTTP 404)" in result.stderr:
                return None
            raise RuntimeError(f"GitHub {method} {path} failed: {result.stderr.strip()}")
        return json.loads(result.stdout) if result.stdout.strip() else None

    def get(self, path, missing=False):
        return self.request("GET", self.prefix + path, missing=missing)

    def post(self, path, data):
        return self.request("POST", self.prefix + path, data)

    def file(self, name, ref):
        data = self.get(f"contents/{name}?ref={urllib.parse.quote(ref, safe='')}")
        return base64.b64decode(data["content"]).decode()


def release_for(api, version):
    tag = f"v{version}"
    found = api.get(f"releases/tags/{tag}", missing=True)
    if found:
        return found
    # GitHub documents the tag endpoint for published releases. Drafts must
    # also be found through the authenticated, paginated release listing.
    matches = []
    page = 1
    while True:
        batch = api.get(f"releases?per_page=100&page={page}")
        matches.extend(item for item in batch if item["tag_name"] == tag)
        if len(batch) < 100:
            break
        page += 1
    if len(matches) > 1:
        raise ValueError("Multiple releases use this tag; resolve the duplicate drafts manually")
    return matches[0] if matches else None


def require_draft(release):
    if release and (not release.get("draft") or release.get("immutable")):
        raise ValueError("This version is already published or immutable; choose a new version")


def tag_commit(api, version):
    ref = api.get(f"git/ref/tags/v{version}", missing=True)
    if not ref:
        return None
    obj = ref["object"]
    for _ in range(8):
        if obj["type"] == "commit":
            return obj["sha"]
        if obj["type"] != "tag":
            break
        obj = api.get(f"git/tags/{obj['sha']}")["object"]
    raise ValueError("Release tag does not resolve to a commit")


def prepare(api, version, source, run_id):
    version_parts(version)
    require_draft(release_for(api, version))
    branch = f"release/v{version}"
    message = f"chore: prepare release\n\nSource commit: {source}\nActions run: {run_id}"
    desired = bump_files(api.file("Cargo.toml", source), api.file("Cargo.lock", source), version)
    ref = api.get(f"git/ref/heads/{branch}", missing=True)
    existing_tag = tag_commit(api, version)
    if existing_tag and not ref:
        raise ValueError("Tag already exists without this workflow's release branch")
    if not ref:
        ref = api.post("git/refs", {"ref": f"refs/heads/{branch}", "sha": source})
    head = ref["object"]["sha"]
    if head == source:
        if existing_tag:
            raise ValueError("Tag exists before release preparation; refusing to replace it")
        mutation = """mutation($input: CreateCommitOnBranchInput!) {
          createCommitOnBranch(input: $input) { commit { oid } }
        }"""
        result = api.request("POST", "graphql", {"query": mutation, "variables": {"input": {
            "branch": {"repositoryNameWithOwner": api.repo, "branchName": branch},
            "expectedHeadOid": source,
            "message": {"headline": "chore: prepare release", "body": message.split("\n\n", 1)[1]},
            "fileChanges": {"additions": [{"path": p, "contents": base64.b64encode(v.encode()).decode()}
                                           for p, v in desired.items()]},
        }}})
        if result.get("errors"):
            raise RuntimeError(f"GitHub refused the signed release commit: {result['errors']}")
        head = result["data"]["createCommitOnBranch"]["commit"]["oid"]
    commit = api.get(f"commits/{head}")
    if ([p["sha"] for p in commit["parents"]] != [source]
            or commit["commit"]["message"].rstrip("\n") != message):
        raise ValueError("Release branch belongs to a different source/run. Rerun the original Actions run")
    if not commit["commit"]["verification"]["verified"]:
        raise ValueError("GitHub did not verify the release commit signature")
    if any(f["filename"] not in desired for f in commit.get("files", [])):
        raise ValueError("Release branch contains unexpected file changes")
    if any(api.file(p, head) != content for p, content in desired.items()):
        raise ValueError("Release branch contents differ from the requested version bump")
    if existing_tag and existing_tag != head:
        raise ValueError("Existing tag points at a different commit")
    return head


def validate_assets(directory, version, commit):
    directory = Path(directory)
    expected = set(asset_names(version))
    found = set()
    for target in TARGETS:
        data = json.loads((directory / f"{target}.json").read_text())
        if data["version"] != version or data["commit"] != commit or data["target"] != target:
            raise ValueError(f"Wrong version/commit/target in {target} build manifest")
        if set(data["assets"]) != set(asset_names(version, target)):
            raise ValueError(f"Wrong artifact set for {target}")
        for name, info in data["assets"].items():
            path = directory / name
            if not path.is_file() or path.is_symlink() or path.stat().st_size <= 0:
                raise ValueError(f"Missing or empty asset: {name}")
            if info != {"sha256": digest(path), "size": path.stat().st_size}:
                raise ValueError(f"Asset does not match build manifest: {name}")
            found.add(name)
    if found != expected:
        raise ValueError("Incomplete release")
    return {name: {"sha256": digest(directory / name), "size": (directory / name).stat().st_size}
            for name in sorted(expected)}


def remote_assets(api, release):
    assets = []
    if release:
        page = 1
        while True:
            batch = api.get(f"releases/{release['id']}/assets?per_page=100&page={page}")
            assets.extend(batch)
            if len(batch) < 100:
                break
            page += 1
    return assets


def missing_assets(api, release, directory, expected):
    require_draft(release)
    missing = set(expected)
    for asset in remote_assets(api, release):
        name = asset["name"]
        if name not in expected or name not in missing:
            raise ValueError(f"Unexpected/duplicate release asset: {name}")
        if asset["state"] == "starter" and asset["size"] == 0:
            # GitHub can leave a zero-byte starter after a failed upload.
            api.request("DELETE", api.prefix + f"releases/assets/{asset['id']}")
            continue
        if asset["state"] != "uploaded":
            raise ValueError(f"Release asset is not fully uploaded: {name}")
        local = directory / name
        remote_digest = asset.get("digest")
        if not remote_digest:
            result = subprocess.run(["gh", "api", api.prefix + f"releases/assets/{asset['id']}",
                                     "-H", "Accept: application/octet-stream"], check=True, capture_output=True)
            remote_digest = "sha256:" + hashlib.sha256(result.stdout).hexdigest()
        if asset["size"] != local.stat().st_size or remote_digest != "sha256:" + digest(local):
            raise ValueError(f"Existing asset differs; refusing to replace it: {name}")
        missing.remove(name)
    return sorted(missing)


def write_config(directory, version, names, config_path, upload_checksums=True):
    config = json.loads((ROOT / "jreleaser.json").read_text())
    config["project"]["version"] = version
    config["files"] = {"artifacts": [{"path": str((directory / name).resolve())} for name in names]}
    github = config["release"]["github"]
    if os.environ.get("GITHUB_REPOSITORY"):
        github["owner"], github["name"] = os.environ["GITHUB_REPOSITORY"].split("/", 1)
    github["prerelease"] = {"enabled": bool(version_parts(version)[1])}
    github["checksums"] = upload_checksums
    config_path.write_text(json.dumps(config, indent=2) + "\n")


def publish(api, version, commit, directory, jreleaser="jreleaser"):
    directory = Path(directory).resolve()
    assets = validate_assets(directory, version, commit)
    require_draft(release_for(api, version))
    if api.get(f"git/ref/heads/release/v{version}")["object"]["sha"] != commit:
        raise ValueError("Release branch moved after preparation")
    tag = tag_commit(api, version)
    if tag and tag != commit:
        raise ValueError("Tag points at a different commit")
    if subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() != commit:
        raise ValueError("Publisher must check out the exact release commit")
    manifest = directory / "release-manifest.json"
    manifest.write_text(json.dumps({"version": version, "commit": commit, "assets": assets,
                                   "macosSigning": "ad-hoc; not notarized", "windowsSigning": "unsigned"},
                                  indent=2, sort_keys=True) + "\n")
    names = sorted(assets) + [manifest.name]
    work = ROOT / "target/release-publish"
    work.mkdir(parents=True, exist_ok=True)
    config = work / "jreleaser.json"
    write_config(directory, version, names, config)
    settings = work / "settings.properties"
    settings.write_text("")
    options = ["-c", str(config), "-b", str(ROOT), "--settings-file", str(settings)]
    subprocess.run([jreleaser, "checksum", *options], check=True)
    checksum = ROOT / "out/jreleaser/checksums/checksums_sha256.txt"
    shutil.copy2(checksum, directory / checksum.name)
    # Verify JReleaser's output against the exact files, before anything is uploaded.
    lines = {}
    for line in (directory / checksum.name).read_text().splitlines():
        if line.strip():
            hashed, name = line.split(maxsplit=1)
            lines[Path(name.lstrip(" *")).name] = hashed
    if lines != {name: digest(directory / name) for name in names}:
        raise ValueError("JReleaser checksums do not match the complete release")
    # JReleaser's iteration order is unspecified. Canonical ordering makes this
    # asset byte-identical when an interrupted publication is resumed.
    (directory / checksum.name).write_text("".join(f"{lines[name]}  {name}\n" for name in sorted(lines)))
    names.append(checksum.name)
    release = release_for(api, version)
    pending = missing_assets(api, release, directory, names)
    if not tag:
        api.post("git/refs", {"ref": f"refs/tags/v{version}", "sha": commit})
    # Materialize the verified tag locally for JReleaser's changelog calculation.
    subprocess.run(["git", "fetch", "origin", f"refs/tags/v{version}:refs/tags/v{version}"], check=True)
    if pending or not release:
        # Only missing assets are offered to JReleaser. Existing files and edited
        # draft notes are preserved; partial-run checksums are never uploaded.
        write_config(directory, version, pending, config, upload_checksums=False)
        require_draft(release_for(api, version))
        subprocess.run([jreleaser, "release", *options], check=True)
    release = release_for(api, version)
    if not release or missing_assets(api, release, directory, names):
        raise ValueError("Draft upload is incomplete; rerun this workflow")
    if release["tag_name"] != f"v{version}" or release["prerelease"] != bool(version_parts(version)[1]):
        raise ValueError("Draft tag or prerelease designation does not match the requested version")
    return release


def sync_pr(api, version, release_url):
    owner = api.repo.split("/")[0]
    branch = f"release/v{version}"
    existing = api.get("pulls?state=all&base=main&head=" + urllib.parse.quote(f"{owner}:{branch}", safe=""))
    if existing:
        return existing[0]["html_url"]
    run_url = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{api.repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    pr = api.post("pulls", {"title": f"Prepare Rust Commander {version}", "head": branch, "base": "main",
                            "body": f"Synchronize the tested release version with main.\n\nDraft: {release_url}\n\n"
                            f"Build and package validation: {run_url}\n\n"
                            "Review the downloads and merge this version change before publishing. "
                            "The release tag continues to identify the tested release commit."})
    return pr["html_url"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "publish", "verify-assets"])
    parser.add_argument("--version", default=os.environ.get("RELEASE_VERSION"), required=False)
    parser.add_argument("--commit", default=os.environ.get("RELEASE_COMMIT"))
    parser.add_argument("--directory", type=Path, default=ROOT / "dist/release")
    args = parser.parse_args()
    version_parts(args.version or "")
    if args.command == "verify-assets":
        validate_assets(args.directory, args.version, args.commit)
        return
    api = GitHub(os.environ["GITHUB_REPOSITORY"])
    if args.command == "prepare":
        if os.environ["GITHUB_REF"] != "refs/heads/main":
            raise ValueError("Start releases from main")
        head = prepare(api, args.version, os.environ["GITHUB_SHA"], os.environ["GITHUB_RUN_ID"])
        with open(os.environ["GITHUB_OUTPUT"], "a") as out:
            out.write(f"commit={head}\nversion={args.version}\n")
    else:
        release = publish(api, args.version, args.commit, args.directory)
        url = sync_pr(api, args.version, release["html_url"])
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as out:
            out.write(f"## Release ready for review\n\n[Draft release]({release['html_url']})\n\n"
                      f"[Version synchronization PR]({url})\n\nCommit: `{args.commit}`\n\n"
                      "All six packages and checksums are uploaded. Publish manually after review.\n")


if __name__ == "__main__":
    main()
