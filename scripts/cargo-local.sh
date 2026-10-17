#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -x "$project_dir/.tools/cargo/bin/cargo" ]; then
    export CARGO_HOME="$project_dir/.tools/cargo"
    export RUSTUP_HOME="$project_dir/.tools/rustup"
    export PATH="$CARGO_HOME/bin:$PATH"
fi
cd "$project_dir"
# Apple's SDK ships libarchive, but does not provide a pkg-config descriptor.
if [ "$(uname -s)" = Darwin ] && ! pkg-config --exists libarchive 2>/dev/null; then
    mkdir -p "$project_dir/.tools/pkgconfig"
    archive_version=$(bsdtar --version | awk '{for (i=1; i<=NF; i++) if ($i == "libarchive") print $(i+1)}')
    cat > "$project_dir/.tools/pkgconfig/libarchive.pc" <<EOF
Name: libarchive
Description: macOS system libarchive
Version: $archive_version
Libs: -larchive
Cflags:
EOF
    export PKG_CONFIG_PATH="$project_dir/.tools/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi
exec cargo "$@"
