#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DOC="$ROOT_DIR/CLAUDE.md"

grep -Fq '/usr/sbin/screencapture -i -x -r' "$DOC" || {
  echo "CLAUDE.md must preserve the native screencapture architecture" >&2
  exit 1
}

for forbidden in 'CoreGraphics' 'SelectionOverlay' 'selection overlay' 'source ./scripts/signing-env.sh && npm run tauri build'; do
  if grep -Fq "$forbidden" "$DOC"; then
    echo "CLAUDE.md contains obsolete release/screenshot guidance: $forbidden" >&2
    exit 1
  fi
done

grep -Fq 'scripts/package-macos-community.sh' "$DOC" || {
  echo "CLAUDE.md must point local signed builds at the community packaging gate" >&2
  exit 1
}

echo "project invariant tests passed"
