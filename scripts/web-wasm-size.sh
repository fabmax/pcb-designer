#!/usr/bin/env bash
# Trunk's post-build hook (crates/app/Trunk.toml): the loading screen's progress
# (crates/app/loading.mjs) counts against the wasm size Trunk writes into index.html, but
# Trunk 0.22 takes it before wasm-opt shrinks the file, so the bar would stop short.
# This puts the size of the file that's served there instead.
set -euo pipefail
dir="${TRUNK_STAGING_DIR:?run by trunk}"
wasm=$(ls "$dir"/*_bg.wasm)
size=$(wc -c <"$wasm" | tr -d ' ')
name=$(basename "$wasm")
# `__trunkInitializer(init, '…/NAME_bg.wasm', SIZE, …)`
sed -E "s#(${name}', )[0-9]+#\1${size}#" "$dir/index.html" >"$dir/index.html.new"
grep -q "${name}', ${size}" "$dir/index.html.new" || { echo "web-wasm-size: no wasm size in index.html" >&2; exit 1; }
mv "$dir/index.html.new" "$dir/index.html"
