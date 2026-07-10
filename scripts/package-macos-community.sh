#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-universal-apple-darwin}"
BUILD_NUMBER="${2:-1}"
MODE="${3:-all}"
APP_NAME="TTREE"
BUNDLE_IDENTIFIER="com.samwstu.ttree"
PINNED_SIGNING_IDENTITY="TTREE Open Source Code Signing"
PINNED_CERT_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"
SIGNING_IDENTITY="${MACOS_COMMUNITY_SIGNING_IDENTITY:-$PINNED_SIGNING_IDENTITY}"
CONFIGURED_CERT_SHA1="$(printf '%s' "${MACOS_COMMUNITY_CERT_SHA1:-$PINNED_CERT_SHA1}" | tr -d ':' | tr '[:lower:]' '[:upper:]')"
SIGNING_KEYCHAIN="${MACOS_COMMUNITY_SIGNING_KEYCHAIN:-}"
UPDATER_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-}"
UPDATER_PRIVATE_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-}"
UPDATER_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD

contains_encrypted_key_marker() {
  grep -Eqi '(minisign|rsign) encrypted secret key'
}

updater_key_is_encrypted() {
  if [[ -n "$UPDATER_PRIVATE_KEY" ]]; then
    if printf '%s' "$UPDATER_PRIVATE_KEY" | contains_encrypted_key_marker; then
      return 0
    fi
    printf '%s' "$UPDATER_PRIVATE_KEY" |
      openssl base64 -d -A 2>/dev/null |
      contains_encrypted_key_marker
    return
  fi

  if [[ ! -f "$UPDATER_PRIVATE_KEY_PATH" ]]; then
    return 1
  fi
  if contains_encrypted_key_marker < "$UPDATER_PRIVATE_KEY_PATH"; then
    return 0
  fi
  openssl base64 -d -A -in "$UPDATER_PRIVATE_KEY_PATH" 2>/dev/null |
    contains_encrypted_key_marker
}

if [[ "$SIGNING_IDENTITY" != "$PINNED_SIGNING_IDENTITY" ]]; then
  echo "community signing identity must remain $PINNED_SIGNING_IDENTITY" >&2
  exit 2
fi

if [[ "$CONFIGURED_CERT_SHA1" != "$PINNED_CERT_SHA1" ]]; then
  echo "community certificate fingerprint does not match the pinned TTREE identity" >&2
  exit 2
fi

case "$MODE" in
  all) ;;
  build-only | --build-only) MODE="build-only" ;;
  package-only | --package-only) MODE="package-only" ;;
  *)
    echo "mode must be all, build-only, or package-only" >&2
    exit 2
    ;;
esac

if [[ "$MODE" != "build-only" ]]; then
  if [[ -z "$UPDATER_PRIVATE_KEY" && -z "$UPDATER_PRIVATE_KEY_PATH" ]]; then
    echo "TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH is required" >&2
    exit 2
  fi

  if [[ "${#UPDATER_KEY_PASSWORD}" -lt 16 ]]; then
    echo "TAURI_SIGNING_PRIVATE_KEY_PASSWORD must contain at least 16 characters" >&2
    exit 2
  fi

  if ! updater_key_is_encrypted; then
    echo "updater key must be a passphrase-encrypted rsign/minisign private key" >&2
    exit 2
  fi
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
TAURI_VERSION="$(node -p "require('$ROOT_DIR/src-tauri/tauri.conf.json').version")"
if [[ "$VERSION" != "$TAURI_VERSION" ]]; then
  echo "package.json and tauri.conf.json versions must match" >&2
  exit 2
fi

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
  if [[ "$MOUNTED" -eq 1 ]]; then
    hdiutil detach "$MOUNT_DIR" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

build_unsigned_app() {
  cd "$ROOT_DIR"
  npm run tauri build -- \
    --target "$TARGET" \
    --bundles app,dmg \
    --no-sign \
    --config "$CONFIG_JSON"
}

if [[ "$MODE" == "all" || "$MODE" == "build-only" ]]; then
  build_unsigned_app
fi

if [[ ! -d "$APP_PATH" || ! -x "$APP_PATH/Contents/MacOS/ttree" ]]; then
  echo "Tauri did not produce the expected app bundle: $APP_PATH" >&2
  exit 1
fi

if [[ "$MODE" == "build-only" ]]; then
  printf 'Unsigned macOS app ready: %s\n' "$APP_PATH"
  exit 0
fi

cd "$ROOT_DIR"

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
if [[ -n "$SIGNING_KEYCHAIN" ]]; then
  /usr/bin/security lock-keychain "$SIGNING_KEYCHAIN"
fi
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" "$APP_PATH"

rm -f "$ARCHIVE_PATH" "$ARCHIVE_PATH.sig"
tar -czf "$ARCHIVE_PATH" -C "$MACOS_DIR" "$APP_NAME.app"
if [[ -n "$UPDATER_PRIVATE_KEY" ]]; then
  TAURI_SIGNING_PRIVATE_KEY="$UPDATER_PRIVATE_KEY" \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$UPDATER_KEY_PASSWORD" \
    npx --no-install tauri signer sign "$ARCHIVE_PATH"
else
  TAURI_SIGNING_PRIVATE_KEY_PATH="$UPDATER_PRIVATE_KEY_PATH" \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$UPDATER_KEY_PASSWORD" \
    npx --no-install tauri signer sign "$ARCHIVE_PATH"
fi
test -s "$ARCHIVE_PATH.sig"

EXTRACT_DIR="$WORK_DIR/archive"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" "$EXTRACT_DIR/$APP_NAME.app"

DMG_SCRIPT="$DMG_DIR/bundle_dmg.sh"
DMG_PATH="$DMG_DIR/${APP_NAME}_${VERSION}_${ARCH_LABEL}.dmg"
DMG_STAGE="$WORK_DIR/dmg-stage"
if [[ ! -x "$DMG_SCRIPT" ]]; then
  echo "Tauri did not produce the expected DMG bundling script: $DMG_SCRIPT" >&2
  exit 1
fi

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
"$ROOT_DIR/scripts/verify-macos-community-signature.sh" "$MOUNT_DIR/$APP_NAME.app"
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
  printf 'certificate_sha1=%s\n' "$PINNED_CERT_SHA1"
  printf 'cdhash=%s\n' "$CDHASH"
  printf 'designated_requirement=%s\n' "$DESIGNATED_REQUIREMENT"
} > "$RELEASE_DIR/signature-metadata.txt"

printf 'Community release artifacts: %s\n' "$RELEASE_DIR"
ls -lh "$RELEASE_DIR"
