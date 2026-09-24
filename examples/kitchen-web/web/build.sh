#!/usr/bin/env bash
# The kitchen as a site: the game for WebGPU, its data in one file, the
# page around it. `web/build.sh [OUT]` (default `site/`), then serve OUT
# over http (`python3 -m http.server -d site`) — a page from file:// may
# not fetch.
#
# Needs: the wasm32-unknown-unknown target, wasm-bindgen-cli at the
# version in Cargo.lock, and `runity sync` having built library/.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-site}"
mkdir -p "$OUT"

cargo build --profile web --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir "$OUT" --out-name kitchen \
  target/wasm32-unknown-unknown/web/kitchen_rush_web.wasm
if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    "$OUT/kitchen_bg.wasm" -o "$OUT/kitchen_bg.wasm"
fi
python3 web/pack.py "$OUT/data.bin.gz"
cp web/index.html web/runity-net.js web/pad.js "$OUT/"
# Pages serves folders beginning with _ only without Jekyll.
touch "$OUT/.nojekyll"
ls -la "$OUT"
