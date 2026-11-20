#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -x "$project_dir/.tools/cargo/bin/cargo" ]; then
    export CARGO_HOME="$project_dir/.tools/cargo"
    export RUSTUP_HOME="$project_dir/.tools/rustup"
    export PATH="$CARGO_HOME/bin:$PATH"
fi
cd "$project_dir"
platform=$(uname -s)
case "$platform" in
    MINGW*|MSYS*)
        if [ -x /usr/bin/link.exe ] &&
            "${RUSTC:-rustc}" -vV | grep -q '^host: .*windows-msvc$'; then
            # Keep Git Bash's coreutils link.exe out of rustc's linker lookup.
            cargo_path=$(command -v cargo)
            remaining_path=$PATH:
            cargo_env_path=
            separator=
            while [ -n "$remaining_path" ]; do
                directory=${remaining_path%%:*}
                remaining_path=${remaining_path#*:}
                if [ "${directory:-.}/link.exe" -ef /usr/bin/link.exe ]; then
                    continue
                fi
                cargo_env_path=$cargo_env_path$separator$directory
                separator=:
            done
            PATH=$cargo_env_path exec "$cargo_path" "$@"
        fi
        ;;
esac
# Apple's SDK ships libarchive, but does not provide a pkg-config descriptor.
if [ "$platform" = Darwin ] && ! pkg-config --exists libarchive 2>/dev/null; then
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
