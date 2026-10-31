#!/usr/bin/env bash
# Magistral as a site: the game for WebGPU and the page around it.
# `web/build.sh [OUT]` (default `site/`), then serve OUT over http
# (`python3 -m http.server -d site`). The screen, its CSS and the rules
# are built into the wasm, so there is no data file to fetch.
#
# Needs: the wasm32-unknown-unknown target and wasm-bindgen-cli at the
# version in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-site}"
mkdir -p "$OUT"
cargo build --profile web --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir "$OUT" --out-name magistral \
  target/wasm32-unknown-unknown/web/magistral.wasm
cp web/index.html "$OUT/"
ls -la "$OUT"
