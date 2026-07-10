#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="${1:-}"
TAG_NAME="${2:-}"
REPOSITORY="${3:-ZASENJC/ttree}"

if [ -z "$RELEASE_DIR" ] || [ -z "$TAG_NAME" ] || [ ! -d "$RELEASE_DIR" ]; then
  echo "usage: $0 /path/to/community-release vX.Y.Z [owner/repository]" >&2
  exit 2
fi

VERSION="$(node -p "require('$ROOT_DIR/package.json').version")"
ARCHIVE_NAME="TTREE_universal.app.tar.gz"
ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
SIGNATURE_PATH="$ARCHIVE_PATH.sig"
OUTPUT_PATH="$RELEASE_DIR/latest.json"
DOWNLOAD_URL="https://github.com/$REPOSITORY/releases/download/$TAG_NAME/$ARCHIVE_NAME"
PUB_DATE="$(date -u +'%Y-%m-%dT%H:%M:%S.000Z')"

if [ ! -s "$ARCHIVE_PATH" ] || [ ! -s "$SIGNATURE_PATH" ]; then
  echo "missing updater archive or signature in $RELEASE_DIR" >&2
  exit 1
fi

jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$PUB_DATE" \
  --arg url "$DOWNLOAD_URL" \
  --rawfile signature "$SIGNATURE_PATH" \
  '{
    version: $version,
    notes: "Community-signed open-source build. macOS may require Privacy & Security > Open Anyway on first install.",
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
