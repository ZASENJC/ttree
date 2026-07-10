#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
PACKAGE_SCRIPT="$ROOT_DIR/scripts/package-macos-community.sh"
PINNED_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

require_text() {
  local text="$1"
  if ! grep -Fq -- "$text" "$PACKAGE_SCRIPT"; then
    echo "package script is missing: $text" >&2
    exit 1
  fi
}

require_text "PINNED_CERT_SHA1=\"$PINNED_SHA1\""
require_text 'UPDATER_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-}"'
require_text 'UPDATER_PRIVATE_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-}"'
require_text 'UPDATER_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"'
require_text 'unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD'
require_text 'updater key must be a passphrase-encrypted rsign/minisign private key'
require_text '--sign "$PINNED_CERT_SHA1"'
require_text 'certificate leaf = H\"$PINNED_CERT_SHA1\"'
require_text 'TAURI_SIGNING_PRIVATE_KEY="$UPDATER_PRIVATE_KEY"'
require_text 'TAURI_SIGNING_PRIVATE_KEY_PATH="$UPDATER_PRIVATE_KEY_PATH"'
require_text 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$UPDATER_KEY_PASSWORD"'
require_text 'MODE="${3:-all}"'
require_text 'build-only'
require_text 'package-only'
require_text '/usr/bin/security lock-keychain "$SIGNING_KEYCHAIN"'

capture_line="$(grep -Fnm1 'UPDATER_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-}"' "$PACKAGE_SCRIPT" | cut -d: -f1)"
unset_line="$(grep -Fnm1 'unset TAURI_SIGNING_PRIVATE_KEY' "$PACKAGE_SCRIPT" | cut -d: -f1)"
build_line="$(grep -Fnm1 'npm run tauri build' "$PACKAGE_SCRIPT" | cut -d: -f1)"
sign_line="$(grep -Fnm1 'npx --no-install tauri signer sign' "$PACKAGE_SCRIPT" | cut -d: -f1)"
lock_line="$(grep -Fnm1 '/usr/bin/security lock-keychain' "$PACKAGE_SCRIPT" | cut -d: -f1)"

if [[ ! "$capture_line" -lt "$unset_line" || ! "$unset_line" -lt "$build_line" || ! "$build_line" -lt "$lock_line" || ! "$lock_line" -lt "$sign_line" ]]; then
  echo "updater secrets must be captured, unset, withheld from build, then scoped to signer" >&2
  exit 1
fi

mkdir -p "$TMP_DIR/bin"
cat > "$TMP_DIR/bin/npm" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$MOCK_NPM_LOG"
app="$CARGO_TARGET_DIR/universal-apple-darwin/release/bundle/macos/TTREE.app"
mkdir -p "$app/Contents/MacOS"
printf '#!/usr/bin/env sh\n' > "$app/Contents/MacOS/ttree"
chmod +x "$app/Contents/MacOS/ttree"
EOF
chmod +x "$TMP_DIR/bin/npm"

env -u TAURI_SIGNING_PRIVATE_KEY \
  -u TAURI_SIGNING_PRIVATE_KEY_PATH \
  -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
  PATH="$TMP_DIR/bin:$PATH" \
  CARGO_TARGET_DIR="$TMP_DIR/target" \
  MOCK_NPM_LOG="$TMP_DIR/npm.log" \
  "$PACKAGE_SCRIPT" universal-apple-darwin 1 --build-only >/dev/null

grep -Fq 'run tauri build' "$TMP_DIR/npm.log" || {
  echo "build-only mode did not run the unsigned Tauri build" >&2
  exit 1
}

echo "package-macos-community tests passed"
