#!/usr/bin/env python3
"""Render Settings pages with disposable configuration; build with screenshot.

Defaults cover every page in software/light and FemtoVG/dark. Linux needs Xvfb.
"""
import argparse
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
    parser.add_argument("--output", type=Path, default=project / "target/settings-smoke")
    parser.add_argument("--renderer", choices=["software", "femtovg"])
    parser.add_argument("--scheme", choices=["light", "dark"])
    parser.add_argument("--page", type=int, choices=range(9), action="append")
    parser.add_argument("--language", choices=["English", "German", "Spanish"], default="English")
    parser.add_argument("--font-size", type=int, choices=range(10, 25), default=14)
    parser.add_argument("--exercise", action="store_true", help="Also verify native Apply/Cancel and invalid drafts with disposable configuration")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    profiles = [(args.renderer or "software", args.scheme or "light")]
    if not args.renderer and not args.scheme:
        profiles.append(("femtovg", "dark"))
    with tempfile.TemporaryDirectory(prefix="rust-commander-settings-") as directory:
        root = Path(directory)
        left, right = root / "left", root / "right"
        left.mkdir(); right.mkdir()
        for renderer, scheme in profiles:
            for page in args.page or range(9):
                tag = f"settings-{renderer}-{scheme}-{page}-{args.language.lower()}-{args.font_size}"
                config = root / tag
                config.mkdir()
                (config / "settings.json").write_text(json.dumps({"preferences": {
                    "color_scheme": scheme.title(), "language": args.language,
                    "font_size": args.font_size,
                    "mail": {"host": "smtp.example.org", "port": 587, "sender": "sender@example.org", "username": "sender@example.org"},
                }}))
                target = args.output.resolve() / (tag + ".png")
                target.unlink(missing_ok=True)
                env = os.environ.copy()
                for key in list(env):
                    if key.startswith("RUST_COMMANDER_SCREENSHOT_") or key.startswith("RUST_COMMANDER_ICON_GALLERY_") or key == "RUST_COMMANDER_SMOKE_KEEP_OPEN":
                        env.pop(key, None)
                steps = [[900, "settings-page", str(page)]]
                if page == 3:
                    steps.append([1050, "field", json.dumps(["shortcut-filter", "Copy"])])
                env.update(RUST_COMMANDER_CONFIG_DIR=str(config), SLINT_BACKEND="winit-" + renderer,
                           RUST_COMMANDER_SCREENSHOT_TO=str(target), RUST_COMMANDER_SCREENSHOT_ACTION="Preferences",
                           RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(steps), SLINT_SCALE_FACTOR="1")
                result = subprocess.run([str(args.binary.resolve()), str(left), str(right)], env=env,
                                        capture_output=True, text=True, timeout=30)
                (args.output / (tag + ".log")).write_text(result.stdout + result.stderr)
                if result.returncode:
                    raise RuntimeError(f"{tag}: {result.returncode}: {result.stderr}")
                data = target.read_bytes()
                if data[:8] != b"\x89PNG\r\n\x1a\n" or len(data) < 1000:
                    raise RuntimeError(f"No rendered PNG: {tag}")
                width, height = struct.unpack(">II", data[16:24])
                if width < 800 or height < 400:
                    raise RuntimeError(f"Unexpected dimensions: {width}x{height}")
                print(f"{tag}: {width}x{height} passed", flush=True)
        if args.exercise:
            for renderer, scheme in profiles:
                exercise(args, renderer, scheme)


def exercise(args, renderer, scheme):
    for case in ["apply-cancel", "invalid-cancel"]:
        with tempfile.TemporaryDirectory(prefix="rc-native-settings-edit-") as directory:
            root = Path(directory)
            config, left, right = [root / name for name in ["config", "left", "right"]]
            for path in [config, left, right]: path.mkdir()
            settings = config / "settings.json"
            settings.write_text(json.dumps({"preferences": {"font_size": 14, "color_scheme": scheme.title()}}))
            sequence = [[900, "settings-page", "2"],
                [1800, "native-field", json.dumps(["font", "18" if case == "apply-cancel" else "invalid"])]]
            if case == "apply-cancel":
                sequence += [[2400, "native-action", "apply"],
                    [2900, "native-field", json.dumps(["font", "20"])]]
            else:
                sequence += [[2400, "native-check", "Enter a valid number."],
                    [2700, "settings-page", "0"], [3000, "native-check", "Enter a valid number."]]
            sequence += [[3500, "native-action", "cancel"]]
            tag = f"settings-{renderer}-{scheme}-{case}"
            target = args.output.resolve() / (tag + ".png")
            env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config), SLINT_BACKEND="winit-" + renderer,
                RUST_COMMANDER_SCREENSHOT_TO=str(target), RUST_COMMANDER_SCREENSHOT_ACTION="Preferences",
                RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(sequence), RUST_COMMANDER_SCREENSHOT_DELAY_MS="4300")
            env.pop("RUST_COMMANDER_SMOKE_KEEP_OPEN", None)
            result = subprocess.run([str(args.binary.resolve()), str(left), str(right)], env=env,
                capture_output=True, text=True, timeout=30)
            (args.output / (tag + ".log")).write_text(result.stdout + result.stderr)
            if result.returncode: raise RuntimeError(f"{tag}: {result.stderr}")
            actual = json.loads(settings.read_text())["preferences"]["font_size"]
            assert actual == (18 if case == "apply-cancel" else 14), (tag, actual)
            print(f"{tag}: native draft persistence passed", flush=True)


if __name__ == "__main__":
    main()
