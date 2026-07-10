#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="${1:-}"
TAG_NAME="${2:-}"
REPOSITORY="${3:-ZASENJC/ttree}"

if [[ -z "$RELEASE_DIR" || -z "$TAG_NAME" || ! -d "$RELEASE_DIR" ]]; then
  echo "usage: $0 /path/to/community-release vX.Y.Z [owner/repository]" >&2
  exit 2
fi

if [[ ! "$TAG_NAME" =~ ^v[0-9A-Za-z._-]+$ ]]; then
  echo "invalid release tag: $TAG_NAME" >&2
  exit 2
fi

if [[ ! "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "invalid GitHub repository: $REPOSITORY" >&2
  exit 2
fi

VERSION="$(node -p "require('$ROOT_DIR/package.json').version")"
TAURI_VERSION="$(node -p "require('$ROOT_DIR/src-tauri/tauri.conf.json').version")"
if [[ "$VERSION" != "$TAURI_VERSION" ]]; then
  echo "package.json and tauri.conf.json versions must match" >&2
  exit 1
fi

ARCHIVE_NAME="TTREE_universal.app.tar.gz"
ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
SIGNATURE_PATH="$ARCHIVE_PATH.sig"
OUTPUT_PATH="$RELEASE_DIR/latest.json"
DOWNLOAD_URL="https://github.com/$REPOSITORY/releases/download/$TAG_NAME/$ARCHIVE_NAME"
PUB_DATE="$(date -u +'%Y-%m-%dT%H:%M:%S.000Z')"

if [[ ! -s "$ARCHIVE_PATH" || ! -s "$SIGNATURE_PATH" ]]; then
  echo "missing updater archive or signature in $RELEASE_DIR" >&2
  exit 1
fi

SIGNATURE="$(tr -d '\r\n' < "$SIGNATURE_PATH")"
if [[ -z "$SIGNATURE" ]]; then
  echo "updater signature is empty" >&2
  exit 1
fi

jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$PUB_DATE" \
  --arg url "$DOWNLOAD_URL" \
  --arg signature "$SIGNATURE" \
  '{
    version: $version,
    notes: "Community-signed open-source build. On first install, use Privacy & Security > Open Anyway if macOS blocks TTREE.",
    pub_date: $pub_date,
    platforms: {
      "darwin-aarch64": { signature: $signature, url: $url },
      "darwin-x86_64": { signature: $signature, url: $url },
      "darwin-aarch64-app": { signature: $signature, url: $url },
      "darwin-x86_64-app": { signature: $signature, url: $url }
    }
  }' > "$OUTPUT_PATH"

jq -e '.platforms | length == 4' "$OUTPUT_PATH" >/dev/null
echo "Generated $OUTPUT_PATH"
