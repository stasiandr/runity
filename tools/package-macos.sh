#!/usr/bin/env bash
# Package the `smallworld` example as a double-clickable macOS app bundle.
#
#   tools/package-macos.sh
#
# Builds crates/runity/examples/smallworld.rs in release mode and lays the
# binary out as dist/SmallWorld.app, so it can be launched from Finder with
# no terminal and no environment variables. The scene data is baked into the
# binary via include_str!, so the only thing in Contents/Resources is the
# compiled shader library.
#
# The bundle ships runity.metallib because a machine with no Xcode has no
# Metal compiler, and `newLibraryWithSource:` would have nothing to compile
# with. The runtime still tries the source first — the .metallib is the
# fallback, so the checked-in .metal file stays the one source of truth and
# the binary is never committed.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

app_name="SmallWorld"
bundle_id="com.stasiandr.smallworld"
dist_dir="$repo_root/dist"
app_dir="$dist_dir/$app_name.app"
contents_dir="$app_dir/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"

echo "==> Building $app_name (release)"
cargo build --release --example smallworld

# Respects CARGO_TARGET_DIR / a target-dir override in .cargo/config.toml
# instead of assuming the workspace-relative target/ directory.
target_dir="$(cargo metadata --no-deps --format-version 1 | \
    sed -E 's/.*"target_directory":"([^"]*)".*/\1/')"

echo "==> Assembling $app_dir"
rm -rf "$app_dir"
mkdir -p "$macos_dir" "$resources_dir"

cp "$target_dir/release/examples/smallworld" "$macos_dir/$app_name"

echo "==> Compiling shaders to $resources_dir/runity.metallib"
metal_source="$repo_root/crates/runity-gpu/src/shader.metal"
metal_air="$(mktemp -t runity-shader).air"
trap 'rm -f "$metal_air"' EXIT
# Not fatal. The metallib is the fallback for a machine with no Metal compiler;
# a machine building the bundle without one still produces a bundle that runs,
# because the runtime compiles shader.metal from source first anyway. Warn
# loudly rather than refuse to package.
if xcrun metal -c "$metal_source" -o "$metal_air" -ffast-math=false 2>/dev/null &&
    xcrun metallib "$metal_air" -o "$resources_dir/runity.metallib" 2>/dev/null; then
    echo "    $(wc -c < "$resources_dir/runity.metallib" | tr -d ' ') bytes"
else
    echo "    WARNING: no Metal toolchain here, so the bundle ships without" >&2
    echo "    runity.metallib. It will still run wherever a Metal compiler is" >&2
    echo "    present at runtime. Install one with:" >&2
    echo "        xcodebuild -downloadComponent MetalToolchain" >&2
fi

cat > "$contents_dir/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$app_name</string>
    <key>CFBundleIdentifier</key>
    <string>$bundle_id</string>
    <key>CFBundleExecutable</key>
    <string>$app_name</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <!-- The bundle runs on Metal, which sizes its drawable from
         backingScaleFactor and draws at the display's real resolution. The CPU
         path pins its layer to contentsScale 1.0 either way, so opting in
         costs it nothing and buys the GPU path a sharp 2x window. -->
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

echo "==> Ad-hoc signing (so Gatekeeper does not block a local launch)"
codesign -s - --force "$app_dir"

echo "==> Done: $app_dir"
