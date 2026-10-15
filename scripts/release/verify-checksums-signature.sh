#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
RELEASE_DIR="${1:?release directory required}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
python3 - "$RELEASE_DIR/SHA256SUMS.sig" "$WORK/checksums.sig" <<'PY'
import base64
from pathlib import Path
import sys
Path(sys.argv[2]).write_bytes(base64.b64decode(Path(sys.argv[1]).read_bytes().strip(), validate=True))
PY
minisign -V -q -m "$RELEASE_DIR/SHA256SUMS" -x "$WORK/checksums.sig" -p "$ROOT_DIR/scripts/release/updater.pub"
