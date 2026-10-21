#!/usr/bin/env python3
"""Check the real progress UI against a throttled, disposable HTTP download."""
import http.server
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time


def main():
    project = Path(__file__).resolve().parent.parent
    output = project / "target/progress-smoke"
    output.mkdir(parents=True, exist_ok=True)
    size = 32 * 1024 * 1024

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_HEAD(self):
            self.send_response(200)
            self.send_header("Content-Length", str(size))
            self.end_headers()

        def do_GET(self):
            if self.path == "/":
                body = b"<h1>Index of /</h1><a href='transfer.bin'>transfer.bin</a>"
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(size))
            self.end_headers()
            try:
                for _ in range(size // 65536):
                    self.wfile.write(bytes(65536))
                    self.wfile.flush()
                    time.sleep(0.025)
            except (BrokenPipeError, ConnectionResetError):
                pass  # The isolated screenshot process exits before completion.

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix="rust-commander-progress-") as directory:
            for renderer, scheme in [("software", "Light"), ("femtovg", "Dark")]:
                root = Path(directory) / renderer
                config, left, right = [root / name for name in ["config", "left", "right"]]
                for path in [config, left, right]:
                    path.mkdir(parents=True)
                (config / "settings.json").write_text(json.dumps({"preferences": {"color_scheme": scheme}}))
                sequence = [[500, "navigate", f"http://127.0.0.1:{server.server_port}/"],
                            [2000, "action", "Copy"], [2500, "dialog", "accept"]]
                env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config),
                           SLINT_BACKEND="winit-" + renderer, SLINT_SCALE_FACTOR="1",
                           RUST_COMMANDER_SCREENSHOT_TO=str(output / (renderer + ".png")),
                           RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(sequence),
                           RUST_COMMANDER_SCREENSHOT_DELAY_MS="7500")
                for key in ["RUST_COMMANDER_SMOKE_KEEP_OPEN", "RUST_COMMANDER_SCREENSHOT_ACTION",
                            "RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION"]:
                    env.pop(key, None)
                binary = project / "target/debug" / ("rust-commander.exe" if os.name == "nt" else "rust-commander")
                result = subprocess.run([str(binary), str(left), str(right)], env=env,
                                        capture_output=True, text=True, timeout=35)
                log = result.stdout + result.stderr
                (output / (renderer + ".log")).write_text(log)
                match = re.search(r"Progress snapshot: indeterminate=false value=([0-9.]+) text=(\d+)% speed=(.+/s)", log)
                if result.returncode or not match or not 0.05 < float(match[1]) < 0.95:
                    raise RuntimeError(f"{renderer} did not show an active determinate transfer: {log}")
                print(f"{renderer}: {match[2]}%, {match[3]}", flush=True)
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


if __name__ == "__main__":
    main()
