#!/usr/bin/env python3
"""Render the entire icon catalog and file-source modes with disposable settings.

Build with --features screenshot first. Run under Xvfb on Linux. --quick checks
1x; the default adds 1.5x, 2x and 3x gallery renders for both desktop renderers.
"""
import argparse
import itertools
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zipfile


def main():
    repo = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=repo / "target/debug/rust-commander")
    parser.add_argument("--output", type=Path, default=repo / "target/icon-smoke")
    parser.add_argument("--quick", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-commander-icons-") as directory:
        root = Path(directory)
        left, right = root / "files", root / "other"
        left.mkdir(); right.mkdir()
        (left / "Folder").mkdir()
        for name, text in [("Notes.txt", "Icons\n"), ("Sample.XML", "<sample/>\n"),
                           ("Main.java", "class Main {}\n"), ("main.rs", "fn main() {}\n"),
                           ("Cargo.toml", "[package]\n"), (".env.local", "KEY=value\n"),
                           ("Data.json", "{}\n"), ("Unknown.unregistered-6c9e42ba7d3f", "\0")]:
            (left / name).write_text(text)
        shutil.copyfile(repo / "tests/fixtures/two-pages.pdf", left / "Document.pdf")
        shutil.copyfile(repo / "assets/icons/rust-commander.png", left / "Image.png")
        with zipfile.ZipFile(left / "Archive.tar.gz.zip", "w") as archive:
            archive.writestr("Sample.xml", "<sample/>")
        if os.name != "nt":
            (left / "Link.xml").symlink_to("Sample.XML")

        def render(name, env, directories=()):
            path = args.output.resolve() / (name + ".png")
            path.unlink(missing_ok=True)
            if "RUST_COMMANDER_ICON_GALLERY_TO" in env:
                env["RUST_COMMANDER_ICON_GALLERY_TO"] = str(path)
            else:
                env["RUST_COMMANDER_SCREENSHOT_TO"] = str(path)
            result = subprocess.run([str(args.binary.resolve()), *map(str, directories)], env=env,
                                    capture_output=True, text=True, timeout=45)
            (args.output / (name + ".log")).write_text(result.stdout + result.stderr)
            if result.returncode:
                raise RuntimeError(f"{name}: {result.returncode}: {result.stderr}")
            data = path.read_bytes()
            if data[:8] != b"\x89PNG\r\n\x1a\n" or len(data) < 1000:
                raise RuntimeError(f"No usable rendered PNG: {name}")
            width, height = struct.unpack(">II", data[16:24])
            if width < 800 or height < 400:
                raise RuntimeError(f"Unexpected dimensions: {width}x{height}")
            print(f"{name}: {width}x{height} passed", flush=True)

        base = os.environ.copy()
        for key in list(base):
            if key.startswith("RUST_COMMANDER_SCREENSHOT_") or key.startswith("RUST_COMMANDER_ICON_GALLERY_") or key == "RUST_COMMANDER_SMOKE_KEEP_OPEN":
                base.pop(key)
        scales = ["1"] if args.quick else ["1", "1.5", "2", "3"]
        for renderer, scheme in itertools.product(["software", "femtovg"], ["light", "dark"]):
            for scale in scales:
                env = dict(base, SLINT_BACKEND="winit-" + renderer, SLINT_SCALE_FACTOR=scale,
                           RUST_COMMANDER_ICON_GALLERY_TO="pending", RUST_COMMANDER_ICON_GALLERY_SCHEME=scheme)
                render(f"gallery-{renderer}-{scheme}-{scale}x", env)
            for source in ["system_with_fallback", "built_in"]:
                config = root / f"config-{renderer}-{scheme}-{source}"
                config.mkdir()
                (config / "settings.json").write_text(json.dumps({"show_hidden": True, "preferences": {
                    "file_icon_source": source, "color_scheme": scheme.title()}}))
                env = dict(base, SLINT_BACKEND="winit-" + renderer, SLINT_SCALE_FACTOR="1",
                           RUST_COMMANDER_CONFIG_DIR=str(config))
                if scheme == "dark":
                    env["RUST_COMMANDER_SCREENSHOT_ACTION"] = "MarkAll"
                render(f"files-{renderer}-{scheme}-{source}", env, [left, right])


if __name__ == "__main__":
    main()
