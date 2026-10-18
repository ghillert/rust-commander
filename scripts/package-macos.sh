#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$(uname -s)" != Darwin ]; then
    echo "Run this script on macOS." >&2
    exit 1
fi
"$project_dir/scripts/cargo-local.sh" build --release --locked
bundle_dir="$project_dir/target/release/Rust Commander.app"
mkdir -p "$bundle_dir/Contents/MacOS" "$bundle_dir/Contents/Resources"
cp "$project_dir/target/release/rust-commander" "$bundle_dir/Contents/MacOS/rust-commander"
cp "$project_dir/LICENSE" "$project_dir/NOTICE" "$bundle_dir/Contents/Resources/"
cat > "$bundle_dir/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Rust Commander</string>
<key>CFBundleDisplayName</key><string>Rust Commander</string>
<key>CFBundleIdentifier</key><string>io.github.ghillert.rust-commander</string>
<key>CFBundleExecutable</key><string>rust-commander</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.6.0</string>
<key>CFBundleVersion</key><string>6</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --deep --sign - "$bundle_dir"
printf 'Created %s\n' "$bundle_dir"
