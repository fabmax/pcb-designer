#!/usr/bin/env bash
# Builds ngspice for the browser with Emscripten, from the same pinned source release and
# patches as scripts/build-ngspice.sh, into target/ngspice/wasm32-emscripten/:
#   ngspice.mjs, ngspice.wasm   an ES module factory (MODULARIZE) exporting the shared API
#                               (ngSpice_Init, ngSpice_Circ, ngSpice_Command), FS,
#                               addFunction and the heap views
#   COPYING, VERSION
# Test it with: node scripts/ngspice-wasm-test.mjs
#
# Needs Emscripten (emcc on PATH, or EMSDK pointing at an activated emsdk), make, bison,
# flex, patch and a host C compiler (cmpp, the XSPICE code model preprocessor, runs during
# the build).
#
# How it differs from the desktop build:
# * No threads: built without -pthread, Emscripten's pthread functions are stubs. ngspice
#   still takes the pthread branch (the other one is Windows'); only bg_run would start a
#   thread, so the app runs `run` synchronously in a Web Worker instead.
# * One static module: libtool can't make shared libraries here, so the objects are built
#   without its -shared (STATIC=, libngspice_la_*FLAGS=) into libngspice.a, linked by em++
#   (HICUM is C++).
# * No XSPICE code models yet (they're dlopen'ed .cm files on the desktop), and no ivlng
#   (Icarus Verilog bridge).
# Usage: scripts/build-ngspice-wasm.sh [--force]   (prints the output folder)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/ngspice.env

out="$PWD/target/ngspice/wasm32-emscripten"
if [ "${1:-}" != "--force" ] && [ "$(cat "$out/VERSION" 2>/dev/null)" = "$NGSPICE_VERSION" ]; then
    echo "$out"
    exit 0
fi
if ! command -v emcc >/dev/null; then
    if [ -n "${EMSDK:-}" ] && [ -f "$EMSDK/emsdk_env.sh" ]; then
        . "$EMSDK/emsdk_env.sh" >/dev/null 2>&1
    else
        echo "emcc not found: put Emscripten on PATH or set EMSDK" >&2
        exit 1
    fi
fi

step() { echo "build-ngspice-wasm: $(date +%H:%M:%S) $*" >&2; }
quiet() {
    local log=$1
    shift
    "$@" >"$log" 2>&1 || { tail -60 "$log" >&2; [ "$log" = configure.log ] && tail -40 config.log >&2; exit 1; }
}

work="$PWD/target/ngspice/build-wasm-$NGSPICE_VERSION"
rm -rf "$work" "$out"
mkdir -p "$work"
tarball="$work/ngspice-$NGSPICE_VERSION.tar.gz"
step "downloading ngspice $NGSPICE_VERSION"
curl -sSfL -o "$tarball" \
    "https://downloads.sourceforge.net/project/ngspice/ng-spice-rework/$NGSPICE_VERSION/ngspice-$NGSPICE_VERSION.tar.gz"
if command -v sha256sum >/dev/null; then sum=$(sha256sum "$tarball"); else sum=$(shasum -a 256 "$tarball"); fi
if [ "${sum%% *}" != "$NGSPICE_SHA256" ]; then
    echo "checksum mismatch for $tarball" >&2
    exit 1
fi
tar -xzf "$tarball" -C "$work"
src="$work/ngspice-$NGSPICE_VERSION"
for p in scripts/ngspice-patches/*.patch; do
    patch -s -d "$src" -p1 <"$p"
done

mkdir -p "$work/build"
cd "$work/build"
step "configure"
# XSPICE stays on: ngspice 47 doesn't compile without it (OPT_ENH_RSHUNT), and its event
# and MIF code is what code models will need.
quiet configure.log emconfigure "$src/configure" --host=wasm32-unknown-emscripten \
    --with-ngshared --disable-shared --enable-static --enable-xspice --disable-osdi \
    --disable-openmp --with-fftw3=no --with-readline=no --without-x --disable-debug
# No Icarus Verilog bridge (a shared library of its own).
sed -i.orig 's/^\(SUBDIRS = .*\) verilog /\1 /' src/xspice/Makefile
step "cmpp (host)"
(cd src/xspice/cmpp && quiet ../../../cmpp.log make CC="${CC_FOR_BUILD:-cc}" CFLAGS=-O2 LDFLAGS= LIBS=)
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)
step "make -j$jobs"
quiet make.log emmake make -j"$jobs" STATIC= libngspice_la_CFLAGS= libngspice_la_LDFLAGS=

step "link"
mkdir -p "$out"
# 4 MB of stack (Emscripten's default is 64 kB): ngspice's parser and frontend recurse.
quiet link.log em++ -O2 src/.libs/libngspice.a -o "$out/ngspice.mjs" \
    -sMODULARIZE -sEXPORT_ES6 -sENVIRONMENT=web,worker,node \
    -sALLOW_MEMORY_GROWTH -sALLOW_TABLE_GROWTH -sSTACK_SIZE=4MB -sFORCE_FILESYSTEM \
    -sEXPORTED_FUNCTIONS=_ngSpice_Init,_ngSpice_Circ,_ngSpice_Command,_malloc,_free \
    -sEXPORTED_RUNTIME_METHODS=FS,addFunction,UTF8ToString,stringToNewUTF8,HEAPU8,HEAP32,HEAPF64
cp "$src/COPYING" "$out/COPYING"
echo "$NGSPICE_VERSION" >"$out/VERSION"
cd "$out"
rm -rf "$work"
step "done: $out ($(wc -c <"$out/ngspice.wasm" | tr -d ' ') bytes of wasm)"
echo "$out"
