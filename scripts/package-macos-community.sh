#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-universal-apple-darwin}"
BUILD_NUMBER="${2:-1}"
APP_NAME="TTREE"
BUNDLE_IDENTIFIER="com.samwstu.ttree"
SIGNING_IDENTITY="${MACOS_COMMUNITY_SIGNING_IDENTITY:-}"
CERT_SHA1="$(printf '%s' "${MACOS_COMMUNITY_CERT_SHA1:-}" | tr -d ':' | tr '[:lower:]' '[:upper:]')"
SIGNING_KEYCHAIN="${MACOS_COMMUNITY_SIGNING_KEYCHAIN:-}"

if [ -z "$SIGNING_IDENTITY" ] || [ "${#CERT_SHA1}" -ne 40 ]; then
  echo "MACOS_COMMUNITY_SIGNING_IDENTITY and a 40-digit MACOS_COMMUNITY_CERT_SHA1 are required" >&2
  exit 2
fi

if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]; then
  echo "TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH is required" >&2
  exit 2
fi

case "$BUILD_NUMBER" in
  '' | *[!0-9]*)
    echo "build number must contain digits only" >&2
    exit 2
    ;;
esac

case "$TARGET" in
  universal-apple-darwin)
    ARCH_LABEL="universal"
    ;;
  aarch64-apple-darwin)
    ARCH_LABEL="aarch64"
    ;;
  x86_64-apple-darwin)
    ARCH_LABEL="x64"
    ;;
  *)
    echo "unsupported macOS target: $TARGET" >&2
    exit 2
    ;;
esac

VERSION="$(node -p "require('$ROOT_DIR/package.json').version")"
TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT_DIR/src-tauri/target}"
BUNDLE_ROOT="$TARGET_ROOT/$TARGET/release/bundle"
MACOS_DIR="$BUNDLE_ROOT/macos"
DMG_DIR="$BUNDLE_ROOT/dmg"
APP_PATH="$MACOS_DIR/$APP_NAME.app"
ARCHIVE_PATH="$MACOS_DIR/$APP_NAME.app.tar.gz"
RELEASE_DIR="$TARGET_ROOT/community-release"
CONFIG_JSON="$(printf '{"bundle":{"createUpdaterArtifacts":false,"macOS":{"bundleVersion":"%s"}}}' "$BUILD_NUMBER")"
WORK_DIR="$(mktemp -d /tmp/ttree-community-package.XXXXXX)"
MOUNT_DIR="$WORK_DIR/mount"
MOUNTED=0

cleanup() {
  if [ "$MOUNTED" -eq 1 ]; then
    hdiutil detach "$MOUNT_DIR" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

cd "$ROOT_DIR"
npm run tauri build -- \
  --target "$TARGET" \
  --bundles app,dmg \
  --no-sign \
  --config "$CONFIG_JSON"

if [ ! -d "$APP_PATH" ] || [ ! -x "$APP_PATH/Contents/MacOS/ttree" ]; then
  echo "Tauri did not produce the expected app bundle: $APP_PATH" >&2
  exit 1
fi

REQUIREMENT="=designated => identifier \"$BUNDLE_IDENTIFIER\" and certificate leaf = H\"$CERT_SHA1\""
CODESIGN_ARGS=(
  --force
  --options runtime
  --timestamp=none
  --sign "$SIGNING_IDENTITY"
  --identifier "$BUNDLE_IDENTIFIER"
  --requirements "$REQUIREMENT"
)
if [ -n "$SIGNING_KEYCHAIN" ]; then
  CODESIGN_ARGS+=(--keychain "$SIGNING_KEYCHAIN")
fi

/usr/bin/codesign "${CODESIGN_ARGS[@]}" "$APP_PATH"
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" \
  "$APP_PATH" "$BUNDLE_IDENTIFIER" "$SIGNING_IDENTITY" "$CERT_SHA1"

rm -f "$ARCHIVE_PATH" "$ARCHIVE_PATH.sig"
tar -czf "$ARCHIVE_PATH" -C "$MACOS_DIR" "$APP_NAME.app"
env -u TAURI_SIGNING_PRIVATE_KEY_PATH npx tauri signer sign "$ARCHIVE_PATH"
test -s "$ARCHIVE_PATH.sig"

EXTRACT_DIR="$WORK_DIR/archive"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" \
  "$EXTRACT_DIR/$APP_NAME.app" "$BUNDLE_IDENTIFIER" "$SIGNING_IDENTITY" "$CERT_SHA1"

DMG_SCRIPT="$DMG_DIR/bundle_dmg.sh"
DMG_PATH="$DMG_DIR/${APP_NAME}_${VERSION}_${ARCH_LABEL}.dmg"
DMG_STAGE="$WORK_DIR/dmg-stage"
mkdir -p "$DMG_STAGE"
ditto "$APP_PATH" "$DMG_STAGE/$APP_NAME.app"
rm -f "$DMG_DIR"/*.dmg
"$DMG_SCRIPT" \
  --volname "$APP_NAME" \
  --volicon "$DMG_DIR/icon.icns" \
  --window-size 500 350 \
  --icon-size 128 \
  --icon "$APP_NAME.app" 140 170 \
  --hide-extension "$APP_NAME.app" \
  --app-drop-link 360 170 \
  --no-internet-enable \
  "$DMG_PATH" "$DMG_STAGE"

mkdir -p "$MOUNT_DIR"
hdiutil attach "$DMG_PATH" -nobrowse -readonly -mountpoint "$MOUNT_DIR" >/dev/null
MOUNTED=1
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" \
  "$MOUNT_DIR/$APP_NAME.app" "$BUNDLE_IDENTIFIER" "$SIGNING_IDENTITY" "$CERT_SHA1"
hdiutil detach "$MOUNT_DIR" >/dev/null
MOUNTED=0

rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"
cp "$DMG_PATH" "$RELEASE_DIR/"
cp "$ARCHIVE_PATH" "$RELEASE_DIR/${APP_NAME}_${ARCH_LABEL}.app.tar.gz"
cp "$ARCHIVE_PATH.sig" "$RELEASE_DIR/${APP_NAME}_${ARCH_LABEL}.app.tar.gz.sig"

CDHASH="$(codesign -d --verbose=4 "$APP_PATH" 2>&1 | sed -n 's/^CDHash=//p' | head -n 1)"
DESIGNATED_REQUIREMENT="$(codesign -d -r- "$APP_PATH" 2>&1 | sed -n 's/^designated => //p')"
{
  printf 'version=%s\n' "$VERSION"
  printf 'build_number=%s\n' "$BUILD_NUMBER"
  printf 'target=%s\n' "$TARGET"
  printf 'certificate_sha1=%s\n' "$CERT_SHA1"
  printf 'cdhash=%s\n' "$CDHASH"
  printf 'designated_requirement=%s\n' "$DESIGNATED_REQUIREMENT"
} > "$RELEASE_DIR/signature-metadata.txt"

printf 'Community release artifacts: %s\n' "$RELEASE_DIR"
ls -lh "$RELEASE_DIR"
