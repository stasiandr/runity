#!/bin/sh
# Builds the editor as a macOS app: target/release/Scrap.app.
#
#   tools/studio-app.sh            # release build, then the bundle
#   tools/studio-app.sh --install  # and copy it to /Applications
#
# Started from Finder with no scene, the editor opens the scene it opened
# last, or asks for one.
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p scrap-studio

app=target/release/Scrap.app
version=$(cargo pkgid -p scrap-studio | sed 's/.*[#@]//')
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/scrap-studio "$app/Contents/MacOS/scrap-studio"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Scrap</string>
  <key>CFBundleDisplayName</key><string>Scrap</string>
  <key>CFBundleIdentifier</key><string>dev.scrap.studio</string>
  <key>CFBundleExecutable</key><string>scrap-studio</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
EOF

# Ad-hoc signature: Gatekeeper still asks on another Mac, this one runs it.
codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true

if [ "${1:-}" = "--install" ]; then
  rm -rf /Applications/Scrap.app
  cp -R "$app" /Applications/
  echo "installed /Applications/Scrap.app"
else
  echo "built $app"
fi
