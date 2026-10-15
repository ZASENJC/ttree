#!/usr/bin/env bash
# Prove certificate/private-key usability before the expensive universal build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ttree-preflight.XXXXXX")"
KEYCHAIN=""
cleanup() {
  if [[ -n "$KEYCHAIN" ]]; then
    security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
    rm -rf "$(dirname "$KEYCHAIN")"
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT
KEYCHAIN="$("$ROOT_DIR/scripts/macos/import-certificate.sh")"
APP_PATH="$WORK_DIR/TTREE.app"
mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"
printf 'int main(void) { return 0; }\n' |
  env -u MACOS_COMMUNITY_CERTIFICATE -u MACOS_COMMUNITY_CERTIFICATE_PASSWORD \
    -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD -u TAURI_SIGNING_PRIVATE_KEY_PATH \
    -u TAURI_PRIVATE_KEY -u TAURI_PRIVATE_KEY_PASSWORD -u TAURI_PRIVATE_KEY_PATH \
    xcrun clang -x c -o "$APP_PATH/Contents/MacOS/ttree" -
cp "$ROOT_DIR/assets/icons/status-template.png" "$APP_PATH/Contents/Resources/glyph.png"
cat > "$APP_PATH/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>ttree</string>
<key>CFBundleIdentifier</key><string>com.samwstu.ttree</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
"$ROOT_DIR/scripts/macos/sign-app.sh" "$APP_PATH" "$KEYCHAIN"
echo '[certificate] Preflight signing and identity verification passed'
