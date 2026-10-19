#!/usr/bin/env python3
"""Install the pinned Linux JReleaser CLI on the publication runner."""
import hashlib
import io
import os
from pathlib import Path
import urllib.request
import zipfile

VERSION = "1.26.0"
name = f"jreleaser-native-{VERSION}-linux-x86_64.zip"
base = f"https://github.com/jreleaser/jreleaser/releases/download/v{VERSION}/"
data = urllib.request.urlopen(base + name, timeout=120).read()
checksums = urllib.request.urlopen(base + "checksums_sha256.txt", timeout=60).read().decode()
expected = next(line.split()[0] for line in checksums.splitlines() if line.split()[-1].lstrip("*") == name)
if hashlib.sha256(data).hexdigest() != expected:
    raise RuntimeError("JReleaser download failed SHA-256 verification")
destination = Path(os.environ["RUNNER_TEMP"]) / "jreleaser"
with zipfile.ZipFile(io.BytesIO(data)) as archive:
    for member in archive.namelist():
        if Path(member).is_absolute() or ".." in Path(member).parts:
            raise ValueError("Unsafe JReleaser archive entry")
    archive.extractall(destination)
executable, = destination.rglob("jreleaser")
executable.chmod(0o755)
with open(os.environ["GITHUB_PATH"], "a") as stream:
    stream.write(str(executable.parent) + "\n")
