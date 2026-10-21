#!/usr/bin/env python3
"""Render real HTTP URI navigation in both modes, color schemes and renderers."""
import functools
import http.server
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import threading


def main():
    project = Path(__file__).resolve().parent.parent
    binary = project / "target/debug" / ("rust-commander.exe" if os.name == "nt" else "rust-commander")
    output = project / "target/uri-smoke"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-commander-uri-ui-") as directory:
        root = Path(directory)
        files = root / "files"
        files.mkdir()
        (files / "Folder").mkdir()
        (files / "café notes.txt").write_text("URI navigation fixture")
        (files / "Example.pdf").write_bytes((project / "tests/fixtures/two-pages.pdf").read_bytes())

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_):
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(files)))
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            for renderer in ["software", "femtovg"]:
                for mode in ["Address", "Breadcrumbs"]:
                    for scheme, scale in [("Light", "1"), ("Dark", "2")]:
                        name = f"{renderer}-{mode.lower()}-{scheme.lower()}-{scale}x"
                        config = root / name
                        config.mkdir()
                        (config / "settings.json").write_text(json.dumps({"preferences": {"location_bar": mode, "color_scheme": scheme}}))
                        target = output / (name + ".png")
                        env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config), SLINT_BACKEND="winit-" + renderer,
                            SLINT_SCALE_FACTOR=scale, RUST_COMMANDER_SCREENSHOT_TO=str(target),
                            RUST_COMMANDER_SCREENSHOT_ACTION=f"Navigate:http://127.0.0.1:{server.server_port}/",
                            RUST_COMMANDER_SCREENSHOT_DELAY_MS="2500")
                        env.pop("RUST_COMMANDER_SMOKE_KEEP_OPEN", None)
                        env.pop("RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION", None)
                        result = subprocess.run([str(binary), str(root), str(root)], env=env, capture_output=True, text=True, timeout=30)
                        log = result.stdout + result.stderr
                        (output / (name + ".log")).write_text(log)
                        if result.returncode or f"Location snapshot: HTTP · 3 rows · {mode.lower()}" not in log:
                            raise RuntimeError(f"{name} URI navigation failed: {log}")
                        pixels = target.read_bytes()
                        if pixels[:8] != b"\x89PNG\r\n\x1a\n" or len(pixels) < 1000:
                            raise RuntimeError(f"{target} is not a rendered screenshot")
                        width, height = struct.unpack(">II", pixels[16:24])
                        print(f"{name}: {width}×{height}, 3 remote entries", flush=True)
        finally:
            server.shutdown()
            server.server_close()
            worker.join()


if __name__ == "__main__":
    main()
