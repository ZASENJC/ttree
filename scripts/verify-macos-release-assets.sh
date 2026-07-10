#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="${1:-}"
TAG_NAME="${2:-}"
REPOSITORY="${3:-}"
VERSION="${4:-}"
VERIFY_CONTAINERS="${VERIFY_MACOS_CONTAINERS:-1}"

if [[ -z "$RELEASE_DIR" || ! -d "$RELEASE_DIR" || -z "$TAG_NAME" || -z "$REPOSITORY" || -z "$VERSION" ]]; then
  echo "usage: $0 /path/to/release vX.Y.Z owner/repository X.Y.Z" >&2
  exit 2
fi

case "$VERIFY_CONTAINERS" in
  0 | 1) ;;
  *)
    echo "VERIFY_MACOS_CONTAINERS must be 0 or 1" >&2
    exit 2
    ;;
esac

DMG_NAME="TTREE_${VERSION}_universal.dmg"
ARCHIVE_NAME="TTREE_universal.app.tar.gz"
SIGNATURE_NAME="$ARCHIVE_NAME.sig"
MANIFEST_NAME="latest.json"
DMG_PATH="$RELEASE_DIR/$DMG_NAME"
ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
SIGNATURE_PATH="$RELEASE_DIR/$SIGNATURE_NAME"
MANIFEST_PATH="$RELEASE_DIR/$MANIFEST_NAME"
DOWNLOAD_URL="https://github.com/$REPOSITORY/releases/download/$TAG_NAME/$ARCHIVE_NAME"

for asset in "$DMG_PATH" "$ARCHIVE_PATH" "$SIGNATURE_PATH" "$MANIFEST_PATH"; do
  if [[ ! -s "$asset" ]]; then
    echo "missing or empty release asset: $asset" >&2
    exit 1
  fi
done

SIGNATURE="$(tr -d '\r\n' < "$SIGNATURE_PATH")"
if [[ -z "$SIGNATURE" || "$SIGNATURE" == *[!A-Za-z0-9+/=]* ]]; then
  echo "updater signature is not valid base64 text" >&2
  exit 1
fi

jq -e \
  --arg version "$VERSION" \
  --arg signature "$SIGNATURE" \
  --arg url "$DOWNLOAD_URL" \
  '(.version == $version) and
   ((.platforms | keys | sort) == ["darwin-aarch64", "darwin-aarch64-app", "darwin-x86_64", "darwin-x86_64-app"]) and
   (all(.platforms[]; .signature == $signature and .url == $url))' \
  "$MANIFEST_PATH" >/dev/null

command -v minisign >/dev/null 2>&1 || {
  echo "minisign is required to verify updater signatures" >&2
  exit 1
}

WORK_DIR="$(mktemp -d /tmp/ttree-release-verify.XXXXXX)"
MOUNT_DIR="$WORK_DIR/mount"
MOUNTED=0
cleanup() {
  if [[ "$MOUNTED" -eq 1 ]]; then
    hdiutil detach "$MOUNT_DIR" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

PUBKEY_B64="$(jq -er '.plugins.updater.pubkey' "$ROOT_DIR/src-tauri/tauri.conf.json")"
printf '%s' "$PUBKEY_B64" | openssl base64 -d -A -out "$WORK_DIR/updater.pub"
printf '%s' "$SIGNATURE" | openssl base64 -d -A -out "$WORK_DIR/archive.sig"
minisign -V -q \
  -m "$ARCHIVE_PATH" \
  -x "$WORK_DIR/archive.sig" \
  -p "$WORK_DIR/updater.pub"

if [[ "$VERIFY_CONTAINERS" -eq 1 ]]; then
  if tar -tzf "$ARCHIVE_PATH" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
    echo "updater archive contains an unsafe path" >&2
    exit 1
  fi

  EXTRACT_DIR="$WORK_DIR/archive"
  mkdir -p "$EXTRACT_DIR"
  tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
  "$ROOT_DIR/scripts/verify-macos-community-signature.sh" "$EXTRACT_DIR/TTREE.app"

  mkdir -p "$MOUNT_DIR"
  hdiutil attach "$DMG_PATH" -nobrowse -readonly -mountpoint "$MOUNT_DIR" >/dev/null
  MOUNTED=1
  "$ROOT_DIR/scripts/verify-macos-community-signature.sh" "$MOUNT_DIR/TTREE.app"
  hdiutil detach "$MOUNT_DIR" >/dev/null
  MOUNTED=0
fi

echo "[release-assets] verified $DMG_NAME, $ARCHIVE_NAME, $SIGNATURE_NAME, and $MANIFEST_NAME"
