#!/usr/bin/env python3
"""Render disposable file/PDF/source windows with both supported renderers.
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
    parser.add_argument("--width", type=int, default=1240)
    parser.add_argument("--height", type=int, default=780)
    parser.add_argument("--case", action="append", choices=["files", "pdf", "source", "pretty", "source-selected", "source-scrolled", "source-dark", "source-contrast", "source-find", "source-tools", "source-edit", "native-views", "native-edit", "preview"])
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
        source = left / "source.json"
        original = '{\r\n\t"snow": "雪 and café",\r\n\t"enabled": true,\r\n\t"items": [1, 2, 3],\r\n\t"nested": {"name":"original"}\r\n}\r\n'
        source.write_bytes(original.encode())
        long_source = left / "long-source.rs"
        long_source.write_text("pub fn example() {\n" + "".join(
            f'    let line_{line} = "café 雪";\n' for line in range(400)) + "}\n")
        for renderer, scale, case in itertools.product(args.renderer or ["software", "femtovg"], args.scale or [1],
                args.case or ["files", "pdf", "source", "pretty", "source-selected", "source-dark", "source-contrast", "preview"]):
            source.write_bytes(original.encode())
            (config / "settings.json").write_text(json.dumps({"preferences": {
                "directory_tree": True, "preview": case == "preview",
                "contrast": "High" if case == "source-contrast" else "System",
                "color_scheme": "Dark" if case == "source-dark" else "Light"}}))
            tag = f"{renderer}-{case}" + (f"-scale{scale:g}" if scale != 1 else "")
            if (args.width, args.height) != (1240, 780):
                tag += f"-{args.width}x{args.height}"
            target = args.output.resolve() / f"{tag}.png"
            target.unlink(missing_ok=True)
            env = os.environ.copy()
            for key in ["RUST_COMMANDER_SCREENSHOT_ACTION", "RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION",
                        "RUST_COMMANDER_SCREENSHOT_SEQUENCE", "RUST_COMMANDER_SMOKE_KEEP_OPEN"]:
                env.pop(key, None)
            env.update(RUST_COMMANDER_CONFIG_DIR=str(config), SLINT_BACKEND="winit-" + renderer,
                       RUST_COMMANDER_SCREENSHOT_TO=str(target), SLINT_SCALE_FACTOR=str(scale))
            if case == "pdf":
                env["RUST_COMMANDER_SCREENSHOT_ACTION"] = "Open:" + str(pdf)
                env["RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION"] = "rotate"
            elif case == "preview":
                env["RUST_COMMANDER_SCREENSHOT_SEQUENCE"] = json.dumps([[750, "click", str(source)]])
            elif case in ["native-views", "native-edit"]:
                def native(delay, control, value=""):
                    return [delay, "native-viewer", json.dumps([control, value])]
                def check(delay, **expected):
                    return [delay, "viewer-assert", json.dumps(expected)]
                if case == "native-views":
                    env["RUST_COMMANDER_SCREENSHOT_ACTION"] = "Open:" + str(source)
                    steps = [native(1200, "mode", "Pretty view"), check(1400, native=True, mode="Pretty view", dirty=False),
                             native(1500, "mode", "Plain original"), check(1700, mode="Plain original", dirty=False),
                             native(1800, "mode", "Highlighted original"), check(2000, mode="Highlighted original"),
                             native(2100, "key", "Command+F"), check(2300, find=True),
                             native(2400, "query", "snow"), native(2500, "find-next"),
                             native(2600, "key", "Return"), native(2700, "assert-query-focus"),
                             native(2800, "document-click"), native(3000, "assert-document-focus"),
                             native(3100, "key", "X"), check(3300, query="snow"),
                             native(3400, "key", "Command+F"), native(3600, "key", "Escape"), check(3800, find=False),
                             native(3900, "tools"), check(4100, tools=True),
                             native(4300, "key", "Escape"), check(4500, tools=False, dirty=False)]
                else:
                    edited = original.replace('"enabled": true', '"enabled": false')
                    steps = [[750, "click", str(source)], [1000, "action", "Edit"],
                             [1150, "viewer-edit", edited], native(1300, "mode", "Pretty view"),
                             check(1500, native=True, mode="Pretty view", dirty=True),
                             native(1650, "save"), check(2400, dirty=False)]
                env["RUST_COMMANDER_SCREENSHOT_SEQUENCE"] = json.dumps(steps)
            elif case.startswith("source") or case == "pretty":
                env["RUST_COMMANDER_SCREENSHOT_ACTION"] = "Open:" + str(source)
                if case == "source-scrolled":
                    env["RUST_COMMANDER_SCREENSHOT_ACTION"] = "Open:" + str(long_source)
                    env["RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION"] = "find:line_350"
                elif case == "pretty":
                    env["RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION"] = "source-mode:Pretty view"
                elif case == "source-selected":
                    start = original.encode().index(b'"snow"')
                    end = original.encode().index(b'"items"')
                    env["RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION"] = "find:" + original.encode()[start:end].decode()
                elif case in ["source-find", "source-tools"]:
                    callback = "viewer-find" if case == "source-find" else "viewer-tools"
                    env["RUST_COMMANDER_SCREENSHOT_SEQUENCE"] = json.dumps([[1250, callback, ""]])
                elif case == "source-edit":
                    env.pop("RUST_COMMANDER_SCREENSHOT_ACTION", None)
                    env["RUST_COMMANDER_SCREENSHOT_SEQUENCE"] = json.dumps([
                        [750, "click", str(source)], [1000, "action", "Edit"],
                        [1250, "viewer", "source-mode:Pretty view"]])
            if (args.width, args.height) != (1240, 780):
                steps = json.loads(env.get("RUST_COMMANDER_SCREENSHOT_SEQUENCE", "[]"))
                steps.insert(0, [250, "resize", json.dumps([args.width, args.height])])
                env["RUST_COMMANDER_SCREENSHOT_SEQUENCE"] = json.dumps(steps)
            result = subprocess.run([str(args.binary.resolve()), str(left), str(right)],
                                    env=env, capture_output=True, text=True, timeout=30)
            (args.output / f"{tag}.log").write_text(result.stdout + result.stderr)
            if result.returncode:
                raise RuntimeError(f"{renderer}/{case} exited {result.returncode}: {result.stderr}")
            data = target.read_bytes()
            if data[:8] != b"\x89PNG\r\n\x1a\n" or len(data) < 1000:
                raise RuntimeError(f"{target} is not a rendered PNG")
            width, height = struct.unpack(">II", data[16:24])
            if (args.width, args.height) != (1240, 780) and (width, height) != (round(args.width * scale), round(args.height * scale)):
                raise RuntimeError(f"Resize did not take effect: {width}x{height}")
            if width < 800 or height < 400:
                raise RuntimeError(f"Unexpected render dimensions: {width}x{height}")
            expected_source = edited if case == "native-edit" else original
            if source.read_bytes() != expected_source.encode():
                raise RuntimeError("Source viewing changed the original file")
            print(f"{tag}: {width}x{height} render passed", flush=True)


if __name__ == "__main__":
    main()
