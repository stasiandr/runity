#!/usr/bin/env bash
# The kitchen as a site: the game for WebGPU, its data in one file, the
# page around it. `web/build.sh [OUT]` (default `site/`), then serve OUT
# over http (`python3 -m http.server -d site`) — a page from file:// may
# not fetch.
#
# Needs: the wasm32-unknown-unknown target, wasm-bindgen-cli at the
# version in Cargo.lock, and `scrap sync` having built library/.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-site}"
mkdir -p "$OUT"

cargo build --profile web --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir "$OUT" --out-name kitchen \
  target/wasm32-unknown-unknown/web/kitchen_rush_web.wasm
# wasm-opt only when asked (WASM_OPT=1): an old one (Ubuntu's binaryen)
# breaks wasm-bindgen's externref table.
if [ "${WASM_OPT:-0}" = 1 ] && command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    "$OUT/kitchen_bg.wasm" -o "$OUT/kitchen_bg.wasm"
fi
python3 web/pack.py "$OUT/data.bin.gz"
cp web/index.html web/pad.js "$OUT/"
# The relay's credentials endpoint, when there is one (SCRAP_TURN_URL: a
# Metered app's REST URL with its key; the Pages workflow has it as a
# secret). It ends up in the page, as anything a static site uses must.
python3 - "$OUT/scrap-net.js" <<'PY'
import os, sys
text = open("web/scrap-net.js").read()
url = os.environ.get("SCRAP_TURN_URL", "")
if url:
    text = text.replace("__SCRAP_TURN_URL__", url)
open(sys.argv[1], "w").write(text)
PY
# Pages serves folders beginning with _ only without Jekyll.
touch "$OUT/.nojekyll"
ls -la "$OUT"
