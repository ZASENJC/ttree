#!/usr/bin/env sh

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "[codesign] verify-macos-signature.sh is deprecated; enforcing the pinned community identity" >&2
exec "$SCRIPT_DIR/verify-macos-community-signature.sh" "${1:-}"
