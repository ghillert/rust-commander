#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$(uname -s)" != Darwin ]; then
    echo "Run this script on macOS." >&2
    exit 1
fi
cd "$project_dir"
export MACOSX_DEPLOYMENT_TARGET=15.0
case "${RUSTFLAGS:-}" in
    *headerpad_max_install_names*) ;;
    *) export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C link-arg=-Wl,-headerpad_max_install_names" ;;
esac
./scripts/cargo-local.sh build --release --locked
case "$(uname -m)" in
    arm64) target=macos-arm64 ;;
    x86_64) target=macos-x64 ;;
    *) echo "Unsupported Mac architecture" >&2; exit 1 ;;
esac
python3 scripts/release/package.py --target "$target" --app-only
