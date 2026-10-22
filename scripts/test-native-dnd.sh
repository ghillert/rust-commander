#!/bin/sh
# Disposable native protocol validation; requires Xvfb, XTEST and Weston.
set -eu
cd "$(dirname "$0")/.."
mkdir -p target/native-dnd-logs
xvfb-run -a -s '-screen 0 1280x800x24 -noreset' \
    cargo test --locked -p rust-commander-native-dnd-tests -- --ignored --test-threads=1
cargo test --locked -p rust-commander-wayland-dnd
runtime=$(mktemp -d)
chmod 700 "$runtime"
export XDG_RUNTIME_DIR="$runtime"
export WAYLAND_DISPLAY=rust-commander-dnd-test
weston --backend=headless-backend.so --socket="$WAYLAND_DISPLAY" --idle-time=0 \
    --log="$PWD/target/native-dnd-logs/weston.log" &
compositor=$!
trap 'kill "$compositor" 2>/dev/null || true; wait "$compositor" 2>/dev/null || true; rm -rf "$runtime"' EXIT HUP INT TERM
python3 - <<'PY'
import os,time
from pathlib import Path
socket=Path(os.environ['XDG_RUNTIME_DIR'])/os.environ['WAYLAND_DISPLAY']
for _ in range(100):
    if socket.is_socket():break
    time.sleep(0.05)
else:raise RuntimeError('The fixture Wayland compositor did not start; inspect target/native-dnd-logs/weston.log')
PY
cargo test --locked -p rust-commander-wayland-dnd \
    worker_shares_a_live_connection_without_protocol_errors -- --ignored --test-threads=1
