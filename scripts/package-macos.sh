#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ALEXANDRIA_PUBKEY:?set ALEXANDRIA_PUBKEY to the release public key hex}"
VERSION="${VERSION:-0.1.0}"
APP="dist/Alexandria.app"
DMG="dist/Alexandria-${VERSION}.dmg"

rustup target add aarch64-apple-darwin x86_64-apple-darwin

echo "building aarch64-apple-darwin"
cargo build --release -p alexandria-app -p alexandria-cli --target aarch64-apple-darwin
echo "building x86_64-apple-darwin"
cargo build --release -p alexandria-app -p alexandria-cli --target x86_64-apple-darwin

rm -rf "$APP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" dist

lipo -create \
  target/aarch64-apple-darwin/release/alexandria-app \
  target/x86_64-apple-darwin/release/alexandria-app \
  -output "$APP/Contents/MacOS/Alexandria"
lipo -create \
  target/aarch64-apple-darwin/release/alex \
  target/x86_64-apple-darwin/release/alex \
  -output "$APP/Contents/MacOS/alex"

cp crates/app/icons/icon.icns "$APP/Contents/Resources/icon.icns"

LLAMA_DIR="${LLAMA_DIR:-target/llama}"
for arch in arm64 x64; do
  [ -x "$LLAMA_DIR/macos-$arch/llama-completion" ] || ./scripts/build-llama.sh "macos-$arch" "$LLAMA_DIR/macos-$arch"
done
lipo -create "$LLAMA_DIR/macos-arm64/llama-completion" "$LLAMA_DIR/macos-x64/llama-completion" \
  -output "$APP/Contents/MacOS/llama-completion"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Alexandria</string>
<key>CFBundleIdentifier</key><string>org.alexandria.kit</string>
<key>CFBundleExecutable</key><string>Alexandria</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>${VERSION}</string>
<key>CFBundleIconFile</key><string>icon.icns</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>11.0</string>
</dict></plist>
PLIST

codesign --force --deep -s - "$APP"
hdiutil create -volname "Alexandria" -srcfolder "$APP" -ov -format UDZO "$DMG" >/dev/null
echo "built $APP"
echo "built $DMG"
