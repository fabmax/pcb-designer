#!/bin/sh
# Builds "PCB Designer.app" (release) into target/release/bundle/.
# Usage: scripts/bundle-macos.sh [--install]   (--install copies it to /Applications)
set -e
cd "$(dirname "$0")/.."
cargo build --release -p app
ngspice=$(scripts/build-ngspice.sh | tail -1)
app="target/release/bundle/PCB Designer.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks" "$app/Contents/PlugIns/ngspice"
cp target/release/pcb-designer "$app/Contents/MacOS/"
# ngspice (sim::bundled_library finds it): the library, its code models, its licence.
cp "$ngspice/lib/libngspice.0.dylib" "$app/Contents/Frameworks/"
cp "$ngspice"/lib/ngspice/*.cm "$app/Contents/PlugIns/ngspice/"
cp "$ngspice/COPYING" "$app/Contents/Resources/ngspice-COPYING.txt"
sed "s|<string>0.1.0</string>|<string>$version</string>|" crates/app/macos/Info.plist > "$app/Contents/Info.plist"
# App icon: every size iconutil wants, scaled from the 1024² PNG.
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
for s in 16 32 128 256 512; do
    sips -z $s $s crates/app/assets/icon.png --out "$iconset/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) crates/app/assets/icon.png --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
# Ad-hoc signatures (local use): ngspice's files first, then the app around them.
for f in "$app"/Contents/Frameworks/*.dylib "$app"/Contents/PlugIns/ngspice/*.cm; do
    codesign --force --sign - "$f"
done
codesign --force --sign - "$app"
echo "built $app"
if [ "$1" = "--install" ]; then
    rm -rf "/Applications/PCB Designer.app"
    cp -R "$app" /Applications/
    echo "installed to /Applications"
fi
