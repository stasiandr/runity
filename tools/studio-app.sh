#!/bin/sh
# Builds the editor as a macOS app: target/release/Scrap.app.
#
#   tools/studio-app.sh            # release build, then the bundle
#   tools/studio-app.sh --install  # and copy it to /Applications
#   tools/studio-app.sh --launcher # ~/Applications/Scrap.app that runs
#                                  # tools/studio-open.sh: rebuilds when the
#                                  # checkout changed, then opens the editor
#
# Started from Finder with no scene, the editor opens the scene it opened
# last, or asks for one.
set -eu
cd "$(dirname "$0")/.."

if [ "${1:-}" = "--launcher" ]; then
  launcher="$HOME/Applications/Scrap.app"
  rm -rf "$launcher"
  mkdir -p "$launcher/Contents/MacOS" "$launcher/Contents/Resources"
  cp crates/scrap-studio/assets/icon/scrap.icns "$launcher/Contents/Resources/scrap.icns"
  printf '#!/bin/sh\nexec "%s/tools/studio-open.sh" "$@"\n' "$(pwd)" > "$launcher/Contents/MacOS/scrap-launcher"
  chmod +x "$launcher/Contents/MacOS/scrap-launcher"
  cat > "$launcher/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Scrap</string>
  <key>CFBundleDisplayName</key><string>Scrap</string>
  <key>CFBundleIdentifier</key><string>dev.scrap.launcher</string>
  <key>CFBundleExecutable</key><string>scrap-launcher</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>scrap</string>
  <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST
  codesign --force --sign - "$launcher" >/dev/null 2>&1 || true
  echo "installed $launcher"
  exit 0
fi

cargo build --release -p scrap-studio

app=target/release/Scrap.app
# A build that changed nothing leaves the bundle as it is (its copy is
# newer than the binary; codesign changes its bytes, so no cmp).
if [ "${1:-}" != "--install" ] && [ "$app/Contents/MacOS/scrap-studio" -nt target/release/scrap-studio ]; then
  echo "up to date $app"
  exit 0
fi
version=$(cargo pkgid -p scrap-studio | sed 's/.*[#@]//')
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/scrap-studio "$app/Contents/MacOS/scrap-studio"
cp crates/scrap-studio/assets/icon/scrap.icns "$app/Contents/Resources/scrap.icns"

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
  <key>CFBundleIconFile</key><string>scrap</string>
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
