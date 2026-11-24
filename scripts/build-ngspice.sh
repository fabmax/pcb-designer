#!/usr/bin/env bash
# Builds libngspice and its XSPICE code models from the pinned source release
# (scripts/ngspice.env), with our patches (scripts/ngspice-patches), into target/ngspice/<rust host triple>/:
#   lib/   libngspice.0.dylib, libngspice.so.0 or ngspice.dll; lib/ngspice/*.cm (code models)
#   COPYING, VERSION
# Releases bundle it (scripts/bundle-*.sh) and CI tests against it (NGSPICE_LIBRARY).
# Runs on macOS, Linux and Windows (MSYS2 UCRT64 shell). Needs a C compiler, make, bison
# flex and patch; the release tarball has its configure script.
# Usage: scripts/build-ngspice.sh [--force]   (prints the output folder)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/ngspice.env

triple=$(rustc -vV | sed -n 's/^host: //p')
out="$PWD/target/ngspice/$triple"
if [ "${1:-}" != "--force" ] && [ "$(cat "$out/VERSION" 2>/dev/null)" = "$NGSPICE_VERSION" ]; then
    echo "$out"
    exit 0
fi

work="$PWD/target/ngspice/build-$NGSPICE_VERSION"
rm -rf "$work" "$out"
mkdir -p "$work"
tarball="$work/ngspice-$NGSPICE_VERSION.tar.gz"
curl -sSfL -o "$tarball" \
    "https://downloads.sourceforge.net/project/ngspice/ng-spice-rework/$NGSPICE_VERSION/ngspice-$NGSPICE_VERSION.tar.gz"
if command -v sha256sum >/dev/null; then sum=$(sha256sum "$tarball"); else sum=$(shasum -a 256 "$tarball"); fi
if [ "${sum%% *}" != "$NGSPICE_SHA256" ]; then
    echo "checksum mismatch for $tarball" >&2
    exit 1
fi
tar -xzf "$tarball" -C "$work"
src="$work/ngspice-$NGSPICE_VERSION"
# Our fixes (scripts/ngspice-patches/*.patch, each saying why).
for p in scripts/ngspice-patches/*.patch; do
    patch -s -d "$src" -p1 <"$p"
done

case "$(uname -s)" in
    Darwin) export MACOSX_DEPLOYMENT_TARGET=11.0 ;;  # as the app (Info.plist)
    MINGW* | MSYS*) export LDFLAGS="${LDFLAGS:-} -static-libgcc" ;;  # no libgcc DLL to ship
esac
# What the app uses: the shared library with XSPICE (code models: POLY sources of vendor
# op amps) and KLU. Off: OSDI (Verilog-A models), OpenMP, FFTW (we do our own spectrum),
# readline and X11 (the interactive program).
mkdir -p "$work/build"
cd "$work/build"
"$src/configure" --prefix="$out" --with-ngshared --enable-xspice --disable-osdi \
    --disable-openmp --with-fftw3=no --with-readline=no --without-x --disable-debug \
    >configure.log
make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" >make.log 2>&1 || { tail -40 make.log >&2; exit 1; }
make install >install.log 2>&1 || { tail -40 install.log >&2; exit 1; }

# Only what the app loads. table.cm is GPL (the rest BSD, LGPL or public domain) and
# loads into the app's process: not shipped.
rm -rf "$out/include" "$out/share" "$out/lib/pkgconfig" "$out/lib/"*.la "$out/lib/"*.a
# ivlng: the Icarus Verilog bridge, unused.
rm -f "$out/lib/ngspice/table.cm" "$out/lib/ngspice/"ivlng.*
case "$(uname -s)" in
    Darwin)
        # Not this build folder: the app loads it by path.
        install_name_tool -id @rpath/libngspice.0.dylib "$out/lib/libngspice.0.dylib" ;;
    MINGW* | MSYS*)
        # libtool puts the DLL in bin/ as libngspice-0.dll; the app looks for
        # lib/ngspice.dll here and ngspice.dll beside its exe.
        mv "$out"/bin/libngspice*.dll "$out/lib/ngspice.dll"
        rm -rf "$out/bin" ;;
esac
cp "$src/COPYING" "$out/COPYING"
echo "$NGSPICE_VERSION" >"$out/VERSION"
echo "$out"
