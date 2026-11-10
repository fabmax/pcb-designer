#!/bin/sh
# Builds "PCB Designer.app" (release) into target/release/bundle/.
# Usage: scripts/bundle-macos.sh [--install]   (--install copies it to /Applications)
set -e
cd "$(dirname "$0")/.."
cargo build --release -p app
app="target/release/bundle/PCB Designer.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/pcb-designer "$app/Contents/MacOS/"
sed "s|<string>0.1.0</string>|<string>$version</string>|" crates/app/macos/Info.plist > "$app/Contents/Info.plist"
codesign --force --sign - "$app" 2>/dev/null || true  # ad-hoc signature (local use)
echo "built $app"
if [ "$1" = "--install" ]; then
    rm -rf "/Applications/PCB Designer.app"
    cp -R "$app" /Applications/
    echo "installed to /Applications"
fi
