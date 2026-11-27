#!/usr/bin/env bash
# Trunk's post-build hook (crates/app/Trunk.toml): puts ngspice into the browser build,
# as ngspice/ beside the app: the Emscripten module (scripts/build-ngspice-wasm.sh's,
# built here the first time when Emscripten is there) and its Web Worker
# (crates/sim/web/ngspice-worker.mjs). Without Emscripten the app is built without
# simulation (it says so when one is run).
set -euo pipefail
cd "$(dirname "$0")/.."
out="${TRUNK_STAGING_DIR:?run by trunk}/ngspice"
built=target/ngspice/wasm32-emscripten
if [ ! -f "$built/VERSION" ]; then
    if command -v emcc >/dev/null || [ -f "${EMSDK:-/nonexistent}/emsdk_env.sh" ]; then
        scripts/build-ngspice-wasm.sh >/dev/null
    else
        echo "web-ngspice: no Emscripten (emcc, EMSDK): building without simulation" >&2
        exit 0
    fi
fi
mkdir -p "$out"
cp "$built/ngspice.mjs" "$built/ngspice.wasm" "$built/COPYING" crates/sim/web/ngspice-worker.mjs "$out/"
