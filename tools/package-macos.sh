#!/usr/bin/env bash
# Package the `smallworld` example as a double-clickable macOS app bundle.
#
#   tools/package-macos.sh
#
# Builds crates/runity/examples/smallworld.rs in release mode and lays the
# binary out as dist/SmallWorld.app, so it can be launched from Finder with
# no terminal and no environment variables. The scene data is baked into the
# binary via include_str!, so Contents/Resources stays empty — there is
# nothing else to ship.
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
    <!-- The renderer works in points, not backing pixels; opting out of
         HiDPI keeps the framebuffer size the code already assumes. -->
    <key>NSHighResolutionCapable</key>
    <false/>
</dict>
</plist>
PLIST

echo "==> Ad-hoc signing (so Gatekeeper does not block a local launch)"
codesign -s - --force "$app_dir"

echo "==> Done: $app_dir"
