#!/usr/bin/env python3
"""Render disposable file/PDF windows with both supported renderers.
Run under xvfb-run on Linux. Uses only temporary app configuration and files.
"""
import argparse
import itertools
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile


def main():
    project = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / ("target/debug/rust-commander.exe" if os.name == "nt" else "target/debug/rust-commander"))
    parser.add_argument("--output", type=Path, default=project / "target/desktop-smoke")
    parser.add_argument("--renderer", action="append", choices=["software", "femtovg"])
    parser.add_argument("--scale", action="append", type=float, choices=[1, 1.25, 1.5, 2])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-commander-desktop-") as directory:
        root = Path(directory)
        left, right, config = [root / name for name in ["left", "right", "config"]]
        for path in [left, right, config]:
            path.mkdir()
        (left / "Documents").mkdir()
        (left / "café notes.txt").write_text("Viewer search fixture\n")
        pdf = left / "Sample.pdf"
        pdf.write_bytes((project / "tests/fixtures/two-pages.pdf").read_bytes())
        (config / "settings.json").write_text(json.dumps({"preferences": {"directory_tree": True}}))
        for renderer, scale, case in itertools.product(args.renderer or ["software", "femtovg"], args.scale or [1], ["files", "pdf"]):
            tag = f"{renderer}-{case}" + (f"-scale{scale:g}" if scale != 1 else "")
            target = args.output.resolve() / f"{tag}.png"
            target.unlink(missing_ok=True)
            env = os.environ.copy()
            for key in ["RUST_COMMANDER_SCREENSHOT_ACTION", "RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION",
                        "RUST_COMMANDER_SMOKE_KEEP_OPEN"]:
                env.pop(key, None)
            env.update(RUST_COMMANDER_CONFIG_DIR=str(config), SLINT_BACKEND="winit-" + renderer,
                       RUST_COMMANDER_SCREENSHOT_TO=str(target), SLINT_SCALE_FACTOR=str(scale))
            if case == "pdf":
                env["RUST_COMMANDER_SCREENSHOT_ACTION"] = "Open:" + str(pdf)
                env["RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION"] = "rotate"
            result = subprocess.run([str(args.binary.resolve()), str(left), str(right)],
                                    env=env, capture_output=True, text=True, timeout=30)
            (args.output / f"{tag}.log").write_text(result.stdout + result.stderr)
            if result.returncode:
                raise RuntimeError(f"{renderer}/{case} exited {result.returncode}: {result.stderr}")
            data = target.read_bytes()
            if data[:8] != b"\x89PNG\r\n\x1a\n" or len(data) < 1000:
                raise RuntimeError(f"{target} is not a rendered PNG")
            width, height = struct.unpack(">II", data[16:24])
            if width < 800 or height < 400:
                raise RuntimeError(f"Unexpected render dimensions: {width}x{height}")
            print(f"{tag}: {width}x{height} render passed", flush=True)


if __name__ == "__main__":
    main()
