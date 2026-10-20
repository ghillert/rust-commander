#!/usr/bin/env python3
"""Build desktop packages from an already-built release executable."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tempfile
import zipfile

from release import ROOT, TARGETS, asset_names, digest, version_parts


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def output(*args):
    return subprocess.check_output([str(a) for a in args], text=True)


def cargo_version():
    command = [os.environ.get("CARGO") or shutil.which("cargo") or str(ROOT / "scripts/cargo-local.sh")]
    metadata = json.loads(output(*command, "metadata", "--no-deps", "--locked", "--format-version", "1",
                                 "--manifest-path", ROOT / "Cargo.toml"))
    return next(package["version"] for package in metadata["packages"]
                if Path(package["manifest_path"]).resolve() == (ROOT / "Cargo.toml").resolve())


def debian_version(version):
    version_parts(version)
    # Debian sorts '~' before a stable version; a SemVer '-' sorts after it.
    return version.replace("-", "~", 1)


def normalize_deb(package, version):
    desired = debian_version(version)
    if desired != version:
        with tempfile.TemporaryDirectory(prefix="rust-commander-deb-") as name:
            extracted = Path(name) / "package"
            run("dpkg-deb", "--raw-extract", package, extracted)
            control = extracted / "DEBIAN/control"
            text, count = re.subn(r"(?m)^Version: .+$", "Version: " + desired, control.read_text())
            if count != 1:
                raise ValueError("Expected exactly one Debian version field")
            control.write_text(text)
            run("dpkg-deb", "--build", "--root-owner-group", extracted, package)
    if output("dpkg-deb", "-f", package, "Version").strip() != desired:
        raise ValueError("Incorrect Debian package version")


def packager_command():
    local = ROOT / ".tools/packager/bin/cargo-packager"
    command = os.environ.get("CARGO_PACKAGER") or shutil.which("cargo-packager")
    if not command and local.exists():
        command = str(local)
    if not command or "0.11.8" not in output(command, "--version"):
        raise RuntimeError("Install cargo-packager 0.11.8 with cargo install --locked")
    return command


def macho_dependencies(path):
    identifiers = output("otool", "-D", path).splitlines()[1:]
    return [dependency for line in output("otool", "-L", path).splitlines()[1:]
            if (dependency := line.strip().split(" (compatibility version", 1)[0]) not in identifiers]


def macho_rpaths(path):
    return re.findall(r"cmd LC_RPATH\n.*?\n\s+path (.*?) \(offset", output("otool", "-l", path))


def system_library(path):
    return str(path).startswith(("/usr/lib/", "/System/Library/"))


def resolve_library(name, owner, executable):
    name = name.replace("@loader_path", str(owner.parent)).replace("@executable_path", str(executable.parent))
    if name.startswith("@rpath/"):
        for search in macho_rpaths(owner) + macho_rpaths(executable):
            search = search.replace("@loader_path", str(owner.parent)).replace("@executable_path", str(executable.parent))
            path = Path(search) / name.removeprefix("@rpath/")
            if path.exists():
                return path.resolve()
        raise ValueError(f"Cannot resolve {name} required by {owner}")
    if not name.startswith("/"):
        raise ValueError(f"Unexpected relative Mach-O dependency: {name}")
    if system_library(name):
        return Path(name)  # System shared-cache files need not exist on disk.
    return Path(name).resolve(strict=True)


def check_macos_baseline(path):
    text = output("otool", "-l", path)
    versions = re.findall(r"\bminos (\d+(?:\.\d+){1,2})", text)
    versions += re.findall(r"cmd LC_VERSION_MIN_MACOSX\n\s+cmdsize \d+\n\s+version (\d+(?:\.\d+){1,2})", text)
    if not versions:
        raise ValueError(f"Missing Mach-O deployment target: {path}")
    if any(tuple(map(int, (v.split(".") + ["0", "0"])[:3])) > (15, 0, 0) for v in versions):
        raise ValueError(f"{path} requires macOS {versions}; rebuild with MACOSX_DEPLOYMENT_TARGET=15.0")


def stage_macos(executable, binary, stage):
    """Copy and relocate the whole non-system dylib closure before signing."""
    frameworks = stage / "Frameworks"
    frameworks.mkdir()
    originals = {executable.resolve(): binary}
    queue = [executable.resolve()]
    edits = []
    while queue:
        source = queue.pop(0)
        dest = originals[source]
        check_macos_baseline(source)
        for dependency in macho_dependencies(source):
            resolved = resolve_library(dependency, source, executable)
            if system_library(resolved) or resolved == source:
                continue
            if resolved not in originals:
                target = frameworks / resolved.name
                if target.exists():
                    raise ValueError(f"Conflicting bundled library names: {target.name}")
                shutil.copy2(resolved, target)
                target.chmod(0o755)
                originals[resolved] = target
                queue.append(resolved)
                # Homebrew keeps license files beside its lib directory.
                licenses = resolved.parent.parent
                for item in licenses.iterdir():
                    if item.is_file() and re.match(r"(LICENSE|COPYING|NOTICE|COPYRIGHT)", item.name, re.I):
                        destination = stage / "third-party-licenses" / resolved.name
                        destination.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(item, destination / item.name)
            prefix = "@executable_path/../Frameworks/" if dest == binary else "@loader_path/"
            edits.append((dest, dependency, prefix + originals[resolved].name))
    for dest in originals.values():
        # Keep the old signature's LINKEDIT allocation while editing. Removing
        # it first leaves a layout that Xcode 16's install_name_tool rejects.
        # cargo-packager replaces these now-invalid signatures after bundling.
        changes = [] if dest == binary else ["-id", "@rpath/" + dest.name]
        for owner, old, new in edits:
            if owner == dest:
                changes.extend(["-change", old, new])
        if changes:
            run("install_name_tool", *changes, dest)
    return sorted(frameworks.glob("*.dylib"))


def windows_runtime(stage):
    run("pwsh", "-NoProfile", "-File", ROOT / "scripts/release/windows-runtime.ps1", "-Destination", stage)
    return sorted(stage.glob("*.dll"))


def debian_package_owners(text):
    # dpkg-query can print diversion notices before the actual ownership line,
    # notably for Ubuntu's merged-/usr dynamic loader. Only accept package names.
    owners = set()
    for line in text.splitlines():
        names, separator, _ = line.partition(": ")
        if separator:
            for name in names.split(", "):
                match = re.fullmatch(r"([a-z0-9][a-z0-9+.-]+)(?::[a-z0-9-]+)?", name)
                if match:
                    owners.add(match[1])
    return owners


def linux_dependencies(binary):
    text = output("ldd", binary)
    if "not found" in text:
        raise ValueError("Missing Linux dependencies: " + text)
    # Derive package names from the actual ELF closure, including t64 names.
    packages = set()
    for line in text.splitlines():
        match = re.search(r"(?:=>\s+)?(/\S+)\s+\(", line)
        if not match:
            continue
        path = Path(match[1])
        candidates = [str(path), str(path.resolve())]
        if str(path).startswith("/lib/"):
            candidates.append("/usr" + str(path))
        owners = set()
        for candidate in candidates:
            result = subprocess.run(["dpkg-query", "-S", candidate], capture_output=True, text=True)
            if result.returncode == 0:
                owners = debian_package_owners(result.stdout)
                if owners:
                    break
        if not owners:
            raise ValueError(f"Cannot map shared library to a Debian package: {path}")
        packages.update(owners)
    # SFTP uses the system OpenSSH client; desktop integration uses xdg-open.
    packages.update(["openssh-client", "xdg-utils", "libglib2.0-bin", "gvfs-backends",
                     "libegl1", "libgl1", "libgtk-4-1",
                     "libxkbcommon0", "libxkbcommon-x11-0", "libwayland-client0",
                     "libx11-6", "libx11-xcb1", "libxcursor1", "libxi6", "libxrandr2"])
    return sorted(packages)


def package(target, app_only=False):
    version = cargo_version()
    core, _ = version_parts(version)
    if target not in TARGETS:
        raise ValueError("Unsupported packaging target")
    expected_os = "Darwin" if target.startswith("macos") else "Windows" if target.startswith("windows") else "Linux"
    if platform.system() != expected_os:
        raise ValueError(f"Build {target} on its native OS")
    arch = platform.machine().lower()
    if (target == "macos-arm64") != (arch in ("arm64", "aarch64")):
        raise ValueError(f"Host architecture {arch} does not match {target}")
    suffix = ".exe" if target.startswith("windows") else ""
    executable = ROOT / f"target/release/rust-commander{suffix}"
    if output(executable, "--version").strip() != f"Rust Commander {version}":
        raise ValueError("Release executable has the wrong version; rebuild it")
    stage = ROOT / "target/release-packaging" / target
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    binary = stage / executable.name
    shutil.copy2(executable, binary)
    for name in ("LICENSE", "NOTICE"):
        shutil.copy2(ROOT / name, stage / name)
    config = {
        "name": "rust-commander", "productName": "Rust Commander", "version": version,
        "identifier": "io.github.ghillert.rust-commander", "publisher": "Gunnar Hillert",
        "description": "A native dual-pane file manager", "category": "Utility",
        "homepage": "https://github.com/ghillert/rust-commander", "authors": ["Gunnar Hillert"],
        "licenseFile": str(ROOT / "LICENSE"), "binaries": [{"path": "rust-commander", "main": True}],
        "binariesDir": str(stage), "outDir": str(stage / "packages"),
        "icons": [str(ROOT / f"assets/icons/rust-commander.{ext}") for ext in ("png", "ico", "icns")],
        "resources": [{"src": str(stage / name), "target": name} for name in ("LICENSE", "NOTICE")],
    }
    if target.startswith("macos"):
        # Keep GPL notices inside the app; do not turn them into a DMG click-
        # through agreement (which also prevents unattended mounting in CI).
        config.pop("licenseFile")
        libraries = stage_macos(executable, binary, stage)
        plist = stage / "Info.plist"
        with plist.open("wb") as stream:
            plistlib.dump({"CFBundleShortVersionString": ".".join(map(str, core)),
                           "CFBundleVersion": ".".join(map(str, core)), "RustCommanderVersion": version,
                           "NSAppleEventsUsageDescription": "Rust Commander uses Finder to open or empty the system Trash when you request these actions.",
                           "NSHighResolutionCapable": True}, stream)
        config["macos"] = {"minimumSystemVersion": "15.0", "signingIdentity": "-",
                           "infoPlistPath": str(plist), "frameworks": [str(p) for p in libraries]}
        config["formats"] = ["app"] if app_only else ["app", "dmg"]
        config["dmg"] = {"windowSize": {"width": 560, "height": 340},
                         "appPosition": {"x": 150, "y": 160}, "appFolderPosition": {"x": 410, "y": 160}}
        licenses = stage / "third-party-licenses"
        if licenses.exists():
            config["resources"].append({"src": str(licenses), "target": "third-party-licenses"})
    elif target.startswith("windows"):
        dlls = windows_runtime(stage)
        config["resources"].extend({"src": str(p), "target": p.name} for p in dlls)
        config["resources"].append({"src": str(stage / "Microsoft-VC-Runtime.txt"), "target": "Microsoft-VC-Runtime.txt"})
        config["formats"] = ["nsis"]
        config["nsis"] = {"installMode": "currentUser", "installerIcon": str(ROOT / "assets/icons/rust-commander.ico")}
        config["windows"] = {"allowDowngrades": False}
    else:
        config["formats"] = ["deb", "appimage"]
        config["deb"] = {"depends": linux_dependencies(binary)}
        # Slint loads window-system/OpenGL libraries dynamically; ldd alone is
        # not enough for a self-contained AppImage. Drivers stay on the host.
        # linuxdeploy's exclusions omit parts of the text-rendering stack,
        # even though the executable links them. Seed that closure explicitly
        # so the AppImage launches on a minimal desktop runtime.
        config["appimage"] = {"libs": ["libfontconfig.so.1", "libfreetype.so.6",
                                        "libharfbuzz.so.0", "libfribidi.so.0",
                                        "libgtk-4.so.1", "libEGL.so.1", "libGL.so.1",
                                        "libxkbcommon.so.0", "libxkbcommon-x11.so.0", "libwayland-client.so.0",
                                        "libX11.so.6", "libX11-xcb.so.1", "libXcursor.so.1", "libXi.so.6", "libXrandr.so.2"]}
    config_file = stage / "packager.json"
    config_file.write_text(json.dumps(config, indent=2) + "\n")
    env = os.environ.copy()
    # This mode is explicitly ad hoc. Ambient developer credentials must not
    # turn local packaging into a notarization/signing submission.
    for key in list(env):
        if key.startswith("APPLE_"):
            del env[key]
    env["APPIMAGE_EXTRACT_AND_RUN"] = "1"
    env["CI"] = "true"  # create-dmg must not depend on interactive Finder automation.
    run(packager_command(), "--config", config_file, "--release", env=env)
    if target.startswith("macos"):
        app = stage / "packages/Rust Commander.app"
        run("codesign", "--verify", "--deep", "--strict", app)
        if app_only:
            dest = ROOT / "target/release/Rust Commander.app"
            if dest.exists():
                shutil.rmtree(dest)
            shutil.copytree(app, dest, symlinks=True)
            print(f"Created {dest}")
            return
    dist = ROOT / "dist/release"
    dist.mkdir(parents=True, exist_ok=True)
    names = asset_names(version, target)
    for name, ext in zip(names, TARGETS[target]):
        dest = dist / name
        if ext == "zip":
            with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                for path in sorted(stage.iterdir()):
                    if path.is_file() and (path.suffix.lower() in (".exe", ".dll") or path.name in ("LICENSE", "NOTICE", "Microsoft-VC-Runtime.txt")):
                        archive.write(path, arcname=path.name)
        else:
            extension = "exe" if ext == "setup.exe" else ext
            matches = list((stage / "packages").rglob(f"*.{extension}"))
            if len(matches) != 1:
                raise ValueError(f"Expected one {extension} package, got {matches}")
            shutil.copy2(matches[0], dest)
            if ext == "deb":
                normalize_deb(dest, version)
    commit = output("git", "-C", ROOT, "rev-parse", "HEAD").strip()
    manifest = {"target": target, "version": version, "commit": commit,
                "assets": {name: {"sha256": digest(dist / name), "size": (dist / name).stat().st_size} for name in names}}
    (dist / f"{target}.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if target == "windows-x64":
        # Exercise a real installer version upgrade without downloading/running
        # older public executables. The fixture contains today's tested binary.
        fixture = ROOT / "target/release-upgrade-fixture"
        if fixture.exists():
            shutil.rmtree(fixture)
        config["version"] = "0.0.0-0"
        config["outDir"] = str(fixture)
        fixture_config = stage / "upgrade-fixture.json"
        fixture_config.write_text(json.dumps(config, indent=2) + "\n")
        run(packager_command(), "--config", fixture_config, "--release", env=env)
    print("Created: " + ", ".join(names))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--app-only", action="store_true")
    arguments = parser.parse_args()
    package(arguments.target, arguments.app_only)
