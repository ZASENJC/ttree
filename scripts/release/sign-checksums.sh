#!/usr/bin/env bash
# Run only after all build jobs and checksum generation have succeeded.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
RELEASE_DIR="${1:?release directory required}"
"$ROOT_DIR/scripts/release/validate-secrets.sh" updater
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
# Tauri signer 2.8.4 requires an extension; sign identical bytes in a .txt file.
cp "$RELEASE_DIR/SHA256SUMS" "$WORK/checksums.txt"
TAURI_PRIVATE_KEY="$TAURI_SIGNING_PRIVATE_KEY" \
  TAURI_PRIVATE_KEY_PASSWORD="$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" \
  npx --yes --package @tauri-apps/cli@2.8.4 tauri signer sign "$WORK/checksums.txt" >/dev/null
cp "$WORK/checksums.txt.sig" "$RELEASE_DIR/SHA256SUMS.sig"
"$ROOT_DIR/scripts/release/verify-checksums-signature.sh" "$RELEASE_DIR"
