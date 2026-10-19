#!/usr/bin/env python3
"""Smoke-test the downloaded/installed packages, using disposable settings."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import time
import zipfile

from release import ROOT, asset_names, digest, version_parts
from package import check_macos_baseline, debian_version, macho_dependencies


def run(*args, **kwargs):
    try:
        return subprocess.run([str(a) for a in args], check=True, **kwargs)
    except subprocess.CalledProcessError as error:
        if error.stderr:
            print(error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else error.stderr)
        raise


def launch(binary, version, work, extra=()):
    env = os.environ.copy()
    env["RUST_COMMANDER_CONFIG_DIR"] = str(work / "settings")
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    for key in list(env):
        if key.startswith("DYLD_"):
            del env[key]
    if hasattr(os, "uname") and os.uname().sysname == "Darwin":
        env["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
    result = run(binary, *extra, "--version", capture_output=True, text=True, env=env)
    if result.stdout.strip() != f"Rust Commander {version}":
        raise ValueError(f"Wrong packaged version: {result.stdout!r}")
    run(binary, *extra, "--help", capture_output=True, env=env)
    left, right = work / "left", work / "right"
    left.mkdir(exist_ok=True); right.mkdir(exist_ok=True)
    with (work / "launch.log").open("w") as log:
        child = subprocess.Popen([str(binary), *extra, str(left), str(right)], env=env, stdout=log, stderr=log)
        try:
            time.sleep(5)
            if child.poll() is not None:
                raise RuntimeError(f"Packaged GUI exited early: {(work / 'launch.log').read_text()}")
        finally:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill(); child.wait()


def macos(directory, version, target, work):
    dmg = directory / asset_names(version, target)[0]
    mounted = plistlib.loads(run("hdiutil", "attach", "-readonly", "-nobrowse", "-plist", dmg, capture_output=True).stdout)
    volume = next(Path(e["mount-point"]) for e in mounted["system-entities"] if "mount-point" in e)
    try:
        if not (volume / "Applications").is_symlink():
            raise ValueError("DMG lacks its Applications shortcut")
        app = work / "Rust Commander.app"
        shutil.copytree(volume / "Rust Commander.app", app, symlinks=True)
    finally:
        run("hdiutil", "detach", volume)
    run("codesign", "--verify", "--deep", "--strict", app)
    plist = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    core = ".".join(map(str, version_parts(version)[0]))
    if (plist["RustCommanderVersion"] != version or plist["LSMinimumSystemVersion"] != "15.0"
            or plist["CFBundleShortVersionString"] != core or plist["CFBundleVersion"] != core):
        raise ValueError("Wrong app bundle version or OS baseline")
    binary = app / "Contents/MacOS/rust-commander"
    expected_arch = "arm64" if target.endswith("arm64") else "x86_64"
    for path in [binary, *app.glob("Contents/Frameworks/*.dylib")]:
        arch = run("lipo", "-archs", path, capture_output=True, text=True).stdout.split()
        if expected_arch not in arch:
            raise ValueError(f"Wrong architecture: {path}")
        check_macos_baseline(path)
        for dep in macho_dependencies(path):
            if not dep.startswith(("/System/Library/", "/usr/lib/", "@loader_path/", "@executable_path/", "@rpath/")):
                raise ValueError(f"Unbundled dependency: {dep}")
    launch(binary, version, work)


def windows(directory, version, work):
    installer, archive = [directory / name for name in asset_names(version, "windows-x64")]
    portable = work / "portable"
    with zipfile.ZipFile(archive) as stream:
        stream.extractall(portable)
    env = dict(os.environ, RELEASE_EXE=str(portable / "rust-commander.exe"))
    metadata = json.loads(run("pwsh", "-NoProfile", "-Command",
        "(Get-Item -LiteralPath $env:RELEASE_EXE).VersionInfo | ConvertTo-Json", env=env,
        capture_output=True, text=True).stdout)
    core = version_parts(version)[0]
    if (metadata["ProductName"] != "Rust Commander" or metadata["ProductVersion"] != version
            or metadata["FileVersion"] != version
            or tuple(metadata[f"File{part}Part"] for part in ("Major", "Minor", "Build")) != core):
        raise ValueError("Wrong Windows executable product/version metadata")
    launch(portable / "rust-commander.exe", version, work)
    # Use the actual default Windows configuration path to test preservation.
    sentinel = Path(os.environ["APPDATA"]) / "ghillert/rust-commander/config/release-smoke-sentinel"
    sentinel.parent.mkdir(parents=True, exist_ok=True)
    if sentinel.exists():
        raise ValueError("Refusing to overwrite an existing smoke-test sentinel")
    sentinel.write_text("preserve settings")
    installed = work / "installed"
    try:
        fixture, = (ROOT / "target/release-upgrade-fixture").rglob("*.exe")
        # Install old fixture metadata, upgrade, then exercise maintenance.
        for candidate in (fixture, installer, installer):
            run(candidate, "/S", f"/D={installed}")
            launch(installed / "rust-commander.exe", version, work)
            if sentinel.read_text() != "preserve settings":
                raise ValueError("Installer changed user configuration")
        run(installed / "uninstall.exe", "/S", f"_?={installed}")
        if (installed / "rust-commander.exe").exists() or sentinel.read_text() != "preserve settings":
            raise ValueError("Uninstall did not remove the program or preserved-settings check failed")
    finally:
        sentinel.unlink(missing_ok=True)


def linux(directory, version, work, kind):
    if kind == "deb":
        deb = directory / asset_names(version, "linux-x64")[1]
        installed_version = run("dpkg-deb", "-f", deb, "Version", capture_output=True, text=True).stdout.strip()
        if installed_version != debian_version(version):
            raise ValueError("Incorrect Debian package version")
        launch(Path("/usr/bin/rust-commander"), version, work)
        return
    image = directory / asset_names(version, "linux-x64")[0]
    image.chmod(0o755)
    launch(image, version, work, ("--appimage-extract-and-run",))
    run(image, "--appimage-extract", cwd=work, stdout=subprocess.DEVNULL)
    launch(work / "squashfs-root/AppRun", version, work)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--directory", type=Path, default=ROOT / "dist/release")
    parser.add_argument("--linux-package", choices=["deb", "appimage"], default="appimage")
    args = parser.parse_args()
    data = json.loads((args.directory / f"{args.target}.json").read_text())
    for name, info in data["assets"].items():
        if digest(args.directory / name) != info["sha256"]:
            raise ValueError(f"Package changed before smoke test: {name}")
    with tempfile.TemporaryDirectory(prefix="rust-commander-package-") as name:
        work = Path(name)
        if args.target.startswith("macos"):
            macos(args.directory, data["version"], args.target, work)
        elif args.target == "windows-x64":
            windows(args.directory, data["version"], work)
        else:
            linux(args.directory, data["version"], work, args.linux_package)
    print(f"Installed-package smoke test passed: {args.target}")


if __name__ == "__main__":
    main()
