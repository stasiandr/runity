#!/bin/sh
# Renders the editor's icon from its two SVGs: scrap-256.png for the window
# (Windows, Linux) and scrap.icns for the Dock. Needs ImageMagick; the .icns
# needs macOS's iconutil. Run from anywhere; writes next to itself.
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# svg size out — full bleed, as a window icon wants it.
render() {
    magick -background none -density $((72 * $2 / 16)) "$1" -resize "$2x$2" "$3"
}
# svg size out — the square inset to 13/16 of the canvas, as macOS draws
# its icons (824 of 1024).
render_mac() {
    inner=$(($2 * 13 / 16))
    render "$1" "$inner" "$tmp/inner.png"
    magick "$tmp/inner.png" -background none -gravity center -extent "$2x$2" "$3"
}

render scrap.svg 256 scrap-256.png

set=$tmp/scrap.iconset
mkdir "$set"
for size in 16 32 128 256 512; do
    double=$((size * 2))
    small=scrap.svg
    [ "$size" -le 32 ] && small=scrap-small.svg
    render_mac "$small" "$size" "$set/icon_${size}x${size}.png"
    [ "$double" -le 32 ] && big=scrap-small.svg || big=scrap.svg
    render_mac "$big" "$double" "$set/icon_${size}x${size}@2x.png"
done
iconutil -c icns -o scrap.icns "$set"
