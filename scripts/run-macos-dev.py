#!/usr/bin/env python3
"""Build and run a local debug .app without the release packaging toolchain."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
CARGO = ROOT / "scripts/cargo-local.sh"


def build():
    command = [str(CARGO), "build", "--locked", "--message-format=json-render-diagnostics"]
    executable = None
    with subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, text=True) as process:
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line, end="", flush=True)
                continue
            if (message.get("reason") == "compiler-artifact"
                    and message.get("target", {}).get("name") == "rust-commander"
                    and message.get("executable")):
                executable = Path(message["executable"])
        if process.wait():
            raise subprocess.CalledProcessError(process.returncode, command)
    if executable is None:
        raise RuntimeError("Cargo did not report the Rust Commander executable")
    metadata = json.loads(subprocess.check_output(
        [str(CARGO), "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT, text=True,
    ))
    package = next(p for p in metadata["packages"]
                   if Path(p["manifest_path"]).resolve() == ROOT / "Cargo.toml")
    return executable, package["version"]


def bundle(executable, version):
    app = executable.parent / "Rust Commander Dev.app"
    with tempfile.TemporaryDirectory(prefix=".rust-commander-dev-", dir=executable.parent) as directory:
        stage = Path(directory) / app.name
        contents = stage / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        resources = contents / "Resources"
        resources.mkdir()
        binary = contents / "MacOS/rust-commander"
        # APFS clones keep the large debug executable cheap to copy. Other file
        # systems fall back to a regular copy, always into a fresh bundle.
        cloned = subprocess.run(["cp", "-c", str(executable), str(binary)], capture_output=True)
        if cloned.returncode:
            shutil.copy2(executable, binary)
        shutil.copy2(ROOT / "assets/icons/rust-commander.icns", resources)
        for name in ("LICENSE", "NOTICE"):
            shutil.copy2(ROOT / name, resources)
        numeric = version.split("-", 1)[0].split("+", 1)[0]
        with (contents / "Info.plist").open("wb") as stream:
            plistlib.dump({
                "CFBundleName": "Rust Commander Dev",
                "CFBundleDisplayName": "Rust Commander Dev",
                "CFBundleIdentifier": "io.github.ghillert.rust-commander.dev",
                "CFBundleExecutable": "rust-commander",
                "CFBundlePackageType": "APPL",
                "CFBundleIconFile": "rust-commander.icns",
                "CFBundleShortVersionString": numeric,
                "CFBundleVersion": numeric,
                "RustCommanderVersion": version,
                "NSHighResolutionCapable": True,
                "NSAppleEventsUsageDescription": "Rust Commander uses Finder to open or empty the system Trash when you request these actions.",
            }, stream)
        subprocess.run(["codesign", "--force", "--sign", "-", str(stage)], check=True)
        subprocess.run(["codesign", "--verify", "--strict", str(stage)], check=True)
        # Renaming preserves the executable inode of an already running dev
        # instance. Never overwrite its mapped executable in place.
        previous = Path(directory) / "previous.app"
        if app.exists():
            app.rename(previous)
        try:
            stage.rename(app)
        except OSError:
            if previous.exists():
                previous.rename(app)
            raise
    return app


def main():
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--build-only", action="store_true")
    options, app_args = parser.parse_known_args()
    if platform.system() != "Darwin":
        parser.error("This app launcher requires macOS; use scripts/run.sh on other platforms")
    if options.build_only and app_args:
        parser.error("--build-only does not accept application arguments")
    os.chdir(ROOT)
    executable, version = build()
    app = bundle(executable, version)
    print(f"Created {app}", flush=True)
    if not options.build_only:
        # Execute from inside the bundle, retaining terminal output, starting
        # paths and environment (including isolated config and Slint overrides).
        binary = str(app / "Contents/MacOS/rust-commander")
        os.execv(binary, [binary, *app_args])


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError, StopIteration) as error:
        print(f"Development launch failed: {error}", file=sys.stderr)
        sys.exit(1)
