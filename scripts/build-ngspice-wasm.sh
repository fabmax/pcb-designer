#!/usr/bin/env bash
# Builds ngspice for the browser with Emscripten, from the same pinned source release and
# patches as scripts/build-ngspice.sh, into target/ngspice/wasm32-emscripten/:
#   *.cm                        the XSPICE code models (side modules)
#   ngspice.mjs, ngspice.wasm   an ES module factory (MODULARIZE) exporting the shared API
#                               (ngSpice_Init, ngSpice_Circ, ngSpice_Command), FS,
#                               addFunction and the heap views
#   COPYING, VERSION
# Test it with: node scripts/ngspice-wasm-test.mjs
#
# Needs Emscripten (emcc on PATH, or EMSDK pointing at an activated emsdk), Node, make,
# bison, flex, patch and a host C compiler (cmpp, the XSPICE code model preprocessor, runs during
# the build).
#
# How it differs from the desktop build:
# * No threads: built without -pthread, Emscripten's pthread functions are stubs. ngspice
#   still takes the pthread branch (the other one is Windows'); only bg_run would start a
#   thread, so the app runs `run` synchronously in a Web Worker instead.
# * One static module: libtool can't make shared libraries here, so the objects are built
#   without its -shared (STATIC=, libngspice_la_*FLAGS=) into libngspice.a, linked by em++
#   (HICUM is C++).
# * XSPICE code models are Emscripten side modules (*.cm, wasm) that ngspice's own
#   dlopen loads from its file system (`codemodel /cm/analog.cm`, the worker writes them
#   there); ngspice is the main module. No ivlng (Icarus Verilog bridge).
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
# XSPICE stays on: ngspice 47 doesn't compile without it (OPT_ENH_RSHUNT), and the code
# models need it. -fPIC: the code models are side modules linked against it at run time.
quiet configure.log emconfigure "$src/configure" --host=wasm32-unknown-emscripten \
    --with-ngshared --disable-shared --enable-static --enable-xspice --disable-osdi \
    --disable-openmp --with-fftw3=no --with-readline=no --without-x --disable-debug \
    CFLAGS="-O2 -fPIC" CXXFLAGS="-O2 -fPIC"
# No Icarus Verilog bridge (a shared library of its own).
sed -i.orig 's/^\(SUBDIRS = .*\) verilog /\1 /' src/xspice/Makefile
step "cmpp (host)"
(cd src/xspice/cmpp && quiet ../../../cmpp.log make CC="${CC_FOR_BUILD:-cc}" CFLAGS=-O2 LDFLAGS= LIBS=)
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)
step "make -j$jobs"
# The code models' link flags go by the host's uname (macOS: -bundle): side modules here.
quiet make.log emmake make -j"$jobs" STATIC= libngspice_la_CFLAGS= libngspice_la_LDFLAGS= \
    LDFLAGS="-sSIDE_MODULE=1 -O2" LIBS=

step "link"
mkdir -p "$out"
# The code models (table.cm is GPL and not shipped, as on the desktop), and what they
# import from ngspice and libc: MAIN_MODULE=2 exports only what's listed.
cms=""
for cm in analog digital spice2poly xtradev xtraevt tlines; do
    cp "src/xspice/icm/$cm/$cm.cm" "$out/"
    cms="$cms $out/$cm.cm"
done
# Their imports (libc; ngspice's own functions they reach through a table it hands
# them): a linked side module has no symbol table for nm, so Node lists them.
# shellcheck disable=SC2086
imports=$(node -e '
const fs = require("fs");
const names = new Set();
for (const f of process.argv.slice(1))
  for (const i of WebAssembly.Module.imports(new WebAssembly.Module(fs.readFileSync(f))))
    if ((i.module === "env" && i.kind === "function") || i.module.startsWith("GOT.")) names.add("_" + i.name);
console.log([...names].sort().join(","));' $cms)
[ -n "$imports" ] || { echo "no imports found in the code models" >&2; exit 1; }
# 4 MB of stack (Emscripten's default is 64 kB): ngspice's parser and frontend recurse.
quiet link.log em++ -O2 src/.libs/libngspice.a -o "$out/ngspice.mjs" \
    -sMAIN_MODULE=2 -sMODULARIZE -sEXPORT_ES6 -sENVIRONMENT=web,worker,node \
    -sALLOW_MEMORY_GROWTH -sALLOW_TABLE_GROWTH -sSTACK_SIZE=4MB -sFORCE_FILESYSTEM \
    -sEXPORTED_FUNCTIONS=_ngSpice_Init,_ngSpice_Circ,_ngSpice_Command,_malloc,_free,"$imports" \
    -sEXPORTED_RUNTIME_METHODS=FS,addFunction,UTF8ToString,stringToNewUTF8,HEAPU8,HEAP32,HEAPF64
cp "$src/COPYING" "$out/COPYING"
echo "$NGSPICE_VERSION" >"$out/VERSION"
cd "$out"
[ -n "${KEEP_WORK:-}" ] || rm -rf "$work"
step "done: $out ($(wc -c <"$out/ngspice.wasm" | tr -d ' ') bytes of wasm)"
echo "$out"
