#!/usr/bin/env python3
"""Render recovery, archive save-back, support and parent navigation using disposable data."""
import argparse
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile


def main():
    project = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/rust-commander")
    parser.add_argument("--output", type=Path, default=project / "target/management-smoke")
    cases=["files", "recovery", "archive", "diagnostics", "report", "bookmarks", "email", "email-settings", "console", "donate", "help", "browser-url", "virtual-pack", "window"]
    parser.add_argument("--case", action="append", choices=cases, dest="cases")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-commander-management-") as directory:
        for renderer in ["software", "femtovg"]:
            for case in args.cases or cases:
                root = Path(directory) / f"{renderer}-{case}"
                config, left, right = [root / name for name in ["config", "left", "right"]]
                for path in [config, left, right]:
                    path.mkdir(parents=True)
                (left / "café notes.txt").write_text("Disposable preview fixture\n")
                (left / "Documents").mkdir()
                (config / "settings.json").write_text(json.dumps({"preferences": {
                    "color_scheme": "Dark" if renderer == "femtovg" else "Light",
                    "parent_entry": True,
                    "donation_url":"https://example.test/support",
                    "mail":{"host":"smtp.example.test","port":587,"sender":"sender@example.test","username":"sender"},
                }, "bookmarks":[{"name":"Working files","location":{"Local":str(left)}},{"name":"Other pane","location":{"Local":str(right)}}]}))
                sequence = []
                if case == "recovery":
                    backup = left / ".rust-commander-replaced-preview"
                    (backup / "items/Reports").mkdir(parents=True)
                    (backup / "items/Reports/previous.txt").write_text("Retained original\n")
                    (backup / "RECOVERY.txt").write_text("Original destination items are kept under items/, with their original relative paths.\n")
                    sequence = [[500, "action", "Recovery"],
                                [900, "field", json.dumps(["recovery-import", str(backup)])],
                                [1200, "dialog", "recovery-import-folder"],
                                [2100, "select", "recovery-item-0"]]
                elif case == "archive":
                    inner = io.BytesIO()
                    with zipfile.ZipFile(inner, "w") as archive:
                        archive.writestr("report.txt", "Editable nested member\n")
                    outer = left / "outer.zip"
                    with zipfile.ZipFile(outer, "w") as archive:
                        archive.writestr("inner.zip", inner.getvalue())
                    sequence = [[400, "open", str(outer)],
                                [1100, "open", "inner.zip"],
                                [2100, "action", "Rename"]]
                elif case in ["diagnostics", "report"]:
                    sequence = [[500, "action", "Diagnostics" if case == "diagnostics" else "ReportBug"]]
                elif case == "bookmarks":
                    sequence=[[500,"action","ExploreBookmarks"],[900,"select","bookmark-item-0"]]
                elif case in ["email","email-settings"]:
                    sequence=[[400,"click",str(left/"café notes.txt")],[700,"action","Email"],[1300,"field",json.dumps(["mail-to","recipient@example.test"])],[1400,"field",json.dumps(["mail-subject","Review selected files"])],[1500,"field",json.dumps(["mail-body","Prepared attachments for review. This preview never sends an email."])]]
                    if case == "email-settings":sequence.append([1800,"dialog","mail-settings"])
                elif case == "console":
                    sequence=[[500,"action","ShowDebugConsole"]]
                elif case == "donate":
                    sequence=[[500,"action","Donate"]]
                elif case == "help":
                    sequence=[[700,"menu","Help"]]
                elif case == "browser-url":
                    sequence=[[500,"action","OpenURLInBrowser"],[900,"field",json.dumps(["browser-url","https://example.test/reports?q=review"])]]
                elif case == "virtual-pack":
                    source=left/"source.zip"
                    with zipfile.ZipFile(source,"w") as archive:
                        archive.writestr("café notes.txt","Selected virtual contents\n")
                    sequence=[[400,"open",str(source)],[1000,"click","café notes.txt"],[1500,"action","Pack"],[1900,"dialog","pack-files"],[2200,"field",json.dumps(["pack-format","7z"])]]
                elif case == "window":
                    sequence=[[700,"menu","Window"]]
                target = args.output.resolve() / f"{renderer}-{case}.png"
                target.unlink(missing_ok=True)
                env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config),
                           SLINT_BACKEND="winit-" + renderer, SLINT_SCALE_FACTOR="1",
                           RUST_COMMANDER_SCREENSHOT_TO=str(target),
                           RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(sequence),
                           RUST_COMMANDER_SCREENSHOT_DELAY_MS="3500")
                for key in ["RUST_COMMANDER_SCREENSHOT_ACTION", "RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION", "RUST_COMMANDER_SMOKE_KEEP_OPEN"]:
                    env.pop(key, None)
                result = subprocess.run([str(args.binary.resolve()), str(left), str(right)],
                                        env=env, capture_output=True, text=True, timeout=60)
                (args.output / f"{renderer}-{case}.log").write_text(result.stdout + result.stderr)
                if result.returncode:
                    raise RuntimeError(f"{renderer}/{case} failed: {result.stderr}")
                data = target.read_bytes()
                if data[:8] != b"\x89PNG\r\n\x1a\n" or len(data) < 1000:
                    raise RuntimeError(f"Missing rendered image: {target}")
                width, height = struct.unpack(">II", data[16:24])
                if width < 800 or height < 400:
                    raise RuntimeError(f"Unexpected dimensions: {width}x{height}")
                print(f"{renderer}/{case}: {width}x{height}", flush=True)


if __name__ == "__main__":
    main()
