#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DOC="$ROOT_DIR/CLAUDE.md"
MACOS_INFO_PLIST="$ROOT_DIR/src-tauri/Info.plist"
TAURI_ENTRYPOINT="$ROOT_DIR/src-tauri/src/lib.rs"

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

if [[ ! -f "$MACOS_INFO_PLIST" ]] ||
  [[ "$(plutil -extract LSUIElement raw -o - "$MACOS_INFO_PLIST" 2>/dev/null || true)" != "true" ]]; then
  echo "macOS bundle must set LSUIElement=true so TTREE stays out of the Dock" >&2
  exit 1
fi

grep -Fq 'set_activation_policy(tauri::ActivationPolicy::Accessory)' "$TAURI_ENTRYPOINT" || {
  echo "macOS startup must use the Accessory activation policy" >&2
  exit 1
}

echo "project invariant tests passed"
