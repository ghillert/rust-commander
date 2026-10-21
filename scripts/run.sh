#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$(uname -s)" = Darwin ]; then
    exec python3 "$project_dir/scripts/run-macos-dev.py" "$@"
fi
exec "$project_dir/scripts/cargo-local.sh" run --locked -- "$@"
