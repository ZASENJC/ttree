#!/usr/bin/env bash
# Build first without secrets; sign and verify the exact containers distributed.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="${1:-universal-apple-darwin}"
BUILD_NUMBER="${2:-1}"
MODE="${3:-all}"
VERSION="$(python3 "$ROOT_DIR/scripts/release/metadata.py")"
TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
APP_DIR="$TARGET_ROOT/$TARGET/release/bundle/macos"
APP_PATH="$APP_DIR/TTREE.app"
RELEASE_DIR="$ROOT_DIR/dist/macos"
UPDATER_KEY="${TAURI_SIGNING_PRIVATE_KEY:-}"
UPDATER_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-}"
UPDATER_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD
unset TAURI_PRIVATE_KEY TAURI_PRIVATE_KEY_PATH TAURI_PRIVATE_KEY_PASSWORD
if [[ -z "$UPDATER_KEY" && -n "$UPDATER_PATH" ]]; then UPDATER_KEY="$(cat "$UPDATER_PATH")"; fi
if printf '%s' "$UPDATER_KEY" | grep -Eqi '(minisign|rsign) encrypted secret key'; then
  UPDATER_KEY="$(printf '%s' "$UPDATER_KEY" | openssl base64 -A)"
fi
case "$BUILD_NUMBER" in ''|*[!0-9]*) echo 'Invalid build number' >&2; exit 2;; esac
case "$MODE" in all|--build-only|--package-only) ;; *) exit 2;; esac
case "$TARGET" in universal-apple-darwin|aarch64-apple-darwin|x86_64-apple-darwin) ;; *) exit 2;; esac
if [[ "$MODE" != "--build-only" && "$TARGET" != universal-apple-darwin ]]; then
  echo 'Distributed macOS assets must be universal' >&2; exit 2
fi
cd "$ROOT_DIR"
if [[ "$MODE" != "--package-only" ]]; then
  if [[ "$TARGET" == universal-apple-darwin ]]; then
    env -u MACOS_COMMUNITY_CERTIFICATE -u MACOS_COMMUNITY_CERTIFICATE_PASSWORD cargo build --locked --release -p ttree-macos --target aarch64-apple-darwin
    env -u MACOS_COMMUNITY_CERTIFICATE -u MACOS_COMMUNITY_CERTIFICATE_PASSWORD cargo build --locked --release -p ttree-macos --target x86_64-apple-darwin
    mkdir -p "$TARGET_ROOT/$TARGET/release"
    lipo -create "$TARGET_ROOT/aarch64-apple-darwin/release/ttree" "$TARGET_ROOT/x86_64-apple-darwin/release/ttree" -output "$TARGET_ROOT/$TARGET/release/ttree"
  else
    env -u MACOS_COMMUNITY_CERTIFICATE -u MACOS_COMMUNITY_CERTIFICATE_PASSWORD cargo build --locked --release -p ttree-macos --target "$TARGET"
  fi
  rm -rf "$APP_PATH"
  mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"
  cp "$TARGET_ROOT/$TARGET/release/ttree" "$APP_PATH/Contents/MacOS/ttree"
  cp assets/icons/icon.icns "$APP_PATH/Contents/Resources/icon.icns"
  /usr/libexec/PlistBuddy -c 'Clear dict' "$APP_PATH/Contents/Info.plist" >/dev/null
  PLIST="$APP_PATH/Contents/Info.plist"
  for pair in 'CFBundleName TTREE' 'CFBundleDisplayName TTREE' 'CFBundleExecutable ttree' 'CFBundleIdentifier com.samwstu.ttree' 'CFBundlePackageType APPL' 'CFBundleIconFile icon.icns' 'NSPrincipalClass NSApplication' 'LSMinimumSystemVersion 11.0'; do
    /usr/libexec/PlistBuddy -c "Add :${pair%% *} string ${pair#* }" "$PLIST"
  done
  /usr/libexec/PlistBuddy -c "Add :CFBundleShortVersionString string $VERSION" "$PLIST"
  /usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $BUILD_NUMBER" "$PLIST"
  /usr/libexec/PlistBuddy -c 'Add :LSUIElement bool true' "$PLIST"
  /usr/libexec/PlistBuddy -c 'Add :NSHighResolutionCapable bool true' "$PLIST"
  /usr/libexec/PlistBuddy -c 'Add :NSScreenCaptureUsageDescription string TTREE 需要屏幕录制权限以完成自由框选截图 OCR 翻译。' "$PLIST"
fi
test -x "$APP_PATH/Contents/MacOS/ttree"
if [[ "$MODE" == "--build-only" ]]; then
  echo "Unsigned verification bundle: $APP_PATH"
  exit 0
fi
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ttree-package.XXXXXX")"
OWN_KEYCHAIN=""
MOUNTED=0
cleanup() {
  if [[ "$MOUNTED" == 1 ]]; then hdiutil detach "$WORK_DIR/mount" >/dev/null 2>&1 || true; fi
  if [[ -n "$OWN_KEYCHAIN" ]]; then
    security delete-keychain "$OWN_KEYCHAIN" >/dev/null 2>&1 || true
    rm -rf "$(dirname "$OWN_KEYCHAIN")"
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT
SIGNING_KEYCHAIN="${MACOS_COMMUNITY_SIGNING_KEYCHAIN:-}"
if [[ -z "$SIGNING_KEYCHAIN" ]]; then
  OWN_KEYCHAIN="$("$ROOT_DIR/scripts/macos/import-certificate.sh")"
  SIGNING_KEYCHAIN="$OWN_KEYCHAIN"
fi
"$ROOT_DIR/scripts/macos/sign-app.sh" "$APP_PATH" "$SIGNING_KEYCHAIN"
security lock-keychain "$SIGNING_KEYCHAIN"
mkdir -p "$RELEASE_DIR"
tar -czf "$WORK_DIR/TTREE_universal.app.tar.gz" -C "$APP_DIR" TTREE.app
TAURI_SIGNING_PRIVATE_KEY="$UPDATER_KEY" TAURI_SIGNING_PRIVATE_KEY_PATH="$UPDATER_PATH" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$UPDATER_PASSWORD" "$ROOT_DIR/scripts/release/validate-secrets.sh" updater
# Only the pinned standalone signer receives updater credentials (no frontend build).
TAURI_PRIVATE_KEY="$UPDATER_KEY" \
  TAURI_PRIVATE_KEY_PASSWORD="$UPDATER_PASSWORD" \
  npx --yes --package @tauri-apps/cli@2.8.4 tauri signer sign "$WORK_DIR/TTREE_universal.app.tar.gz" >/dev/null
mkdir -p "$WORK_DIR/stage"
ditto "$APP_PATH" "$WORK_DIR/stage/TTREE.app"
ln -s /Applications "$WORK_DIR/stage/Applications"
hdiutil create -volname TTREE -srcfolder "$WORK_DIR/stage" -format UDZO -ov "$WORK_DIR/TTREE_${VERSION}_universal.dmg" >/dev/null
cp "$WORK_DIR/TTREE_${VERSION}_universal.dmg" "$WORK_DIR/TTREE_universal.app.tar.gz" "$WORK_DIR/TTREE_universal.app.tar.gz.sig" "$RELEASE_DIR/"
if [[ "$TARGET" == universal-apple-darwin ]]; then
  ARCHS="$(lipo -archs "$APP_PATH/Contents/MacOS/ttree")"
  [[ " $ARCHS " == *" arm64 "* && " $ARCHS " == *" x86_64 "* ]] || { echo 'Universal binary is missing an architecture' >&2; exit 1; }
fi
echo "Signed macOS assets: $RELEASE_DIR"
