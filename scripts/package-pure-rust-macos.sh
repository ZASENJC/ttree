#!/usr/bin/env bash

set -euo pipefail

export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:$PATH"

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="TTREE"
BUNDLE_IDENTIFIER="com.samwstu.ttree"
PINNED_SIGNING_IDENTITY="TTREE Open Source Code Signing"
PINNED_CERT_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"
VERSION="0.2.0"
BUILD_NUMBER="${1:-1}"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BUNDLE_DIR="$TARGET_DIR/release/bundle/macos"
APP_PATH="$BUNDLE_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_PATH/Contents"
MACOS_BIN_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "[1/4] Compiling 100% pure-Rust TTREE release binary..."
cd "$ROOT_DIR"
cargo build --release -p ttree-app

echo "[2/4] Assembling macOS App Bundle at $APP_PATH..."
rm -rf "$APP_PATH"
mkdir -p "$MACOS_BIN_DIR" "$RESOURCES_DIR"

cp "$TARGET_DIR/release/ttree" "$MACOS_BIN_DIR/ttree"
chmod +x "$MACOS_BIN_DIR/ttree"

if [[ -f "$ROOT_DIR/src-tauri/icons/icon.icns" ]]; then
  cp "$ROOT_DIR/src-tauri/icons/icon.icns" "$RESOURCES_DIR/icon.icns"
fi

cat > "$CONTENTS_DIR/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>zh_CN</string>
  <key>CFBundleDisplayName</key>
  <string>${APP_NAME}</string>
  <key>CFBundleExecutable</key>
  <string>ttree</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundleIdentifier</key>
  <string>${BUNDLE_IDENTIFIER}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>${APP_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${BUILD_NUMBER}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSScreenCaptureUsageDescription</key>
  <string>TTREE 需要屏幕录制权限以完成自由框选截图 OCR 翻译。</string>
</dict>
</plist>
EOF

echo "[3/4] Signing $APP_NAME.app with pinned community certificate ($PINNED_CERT_SHA1)..."
TEMP_KEYCHAIN=""
cleanup() {
  if [[ -n "$TEMP_KEYCHAIN" && -f "$TEMP_KEYCHAIN" ]]; then
    security delete-keychain "$TEMP_KEYCHAIN" >/dev/null 2>&1 || rm -f "$TEMP_KEYCHAIN"
  fi
}
trap cleanup EXIT

SIGNING_KEYCHAIN="${MACOS_COMMUNITY_SIGNING_KEYCHAIN:-}"
if [[ -z "$SIGNING_KEYCHAIN" ]]; then
  P12_PATH="$HOME/.config/ttree-code-signing/community-signing.p12"
  if [[ -f "$P12_PATH" ]]; then
    P12_PASS="$(security find-generic-password -s "com.samwstu.ttree" -a "community-code-signing-password" -w)"
    TEMP_KEYCHAIN="$(mktemp -u /tmp/ttree-signing.XXXXXX).keychain-db"
    KC_PASS="$(uuidgen)"
    security create-keychain -p "$KC_PASS" "$TEMP_KEYCHAIN"
    security set-keychain-settings -lut 300 "$TEMP_KEYCHAIN"
    security unlock-keychain -p "$KC_PASS" "$TEMP_KEYCHAIN"
    security import "$P12_PATH" -k "$TEMP_KEYCHAIN" -P "$P12_PASS" -T /usr/bin/codesign -T /usr/bin/security >/dev/null
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$TEMP_KEYCHAIN" >/dev/null
    SIGNING_KEYCHAIN="$TEMP_KEYCHAIN"
  fi
fi

REQUIREMENT="=designated => identifier \"$BUNDLE_IDENTIFIER\" and certificate leaf = H\"$PINNED_CERT_SHA1\""
CODESIGN_ARGS=(
  --force
  --options runtime
  --timestamp=none
  --sign "$PINNED_CERT_SHA1"
  --identifier "$BUNDLE_IDENTIFIER"
  --requirements "$REQUIREMENT"
)
if [[ -n "$SIGNING_KEYCHAIN" ]]; then
  CODESIGN_ARGS+=(--keychain "$SIGNING_KEYCHAIN")
fi

/usr/bin/codesign "${CODESIGN_ARGS[@]}" "$APP_PATH"

echo "[4/4] Verifying macOS community signature & TCC designated requirement..."
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" "$APP_PATH"

printf '\nPure-Rust TTREE.app packaged and verified at: %s\n' "$APP_PATH"
