#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
SIGNING_ENV="$ROOT_DIR/scripts/signing-env.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/bin"
printf '%s\n%s\n' \
  'untrusted comment: minisign encrypted secret key' \
  'mock-key-material' > "$TMP_DIR/private.key"

cat > "$TMP_DIR/bin/security" <<'EOF'
#!/usr/bin/env sh
if [ -n "${MOCK_KEYCHAIN_PASSWORD:-}" ]; then
  printf '%s\n' "$MOCK_KEYCHAIN_PASSWORD"
  exit 0
fi
exit 44
EOF
chmod +x "$TMP_DIR/bin/security"

source_script() {
  env -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    PATH="$TMP_DIR/bin:$PATH" \
    TTREE_UPDATER_KEY_PATH="$TMP_DIR/private.key" \
    MOCK_KEYCHAIN_PASSWORD="${MOCK_KEYCHAIN_PASSWORD:-}" \
    sh -c '. "$1"' _ "$SIGNING_ENV"
}

expect_failure() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "expected failure: $description" >&2
    exit 1
  fi
}

expect_no_key_export_after_failure() {
  env -u TAURI_SIGNING_PRIVATE_KEY \
    -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
    PATH="$TMP_DIR/bin:$PATH" \
    TTREE_UPDATER_KEY_PATH="$TMP_DIR/private.key" \
    MOCK_KEYCHAIN_PASSWORD="" \
    sh -c '. "$1" >/dev/null 2>&1 || true; [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]' \
    _ "$SIGNING_ENV"
}

expect_stale_exports_cleared_after_failure() {
  TAURI_SIGNING_PRIVATE_KEY="stale-key" \
    TAURI_SIGNING_PRIVATE_KEY_PATH="/tmp/stale-key" \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
    PATH="$TMP_DIR/bin:$PATH" \
    TTREE_UPDATER_KEY_PATH="$TMP_DIR/private.key" \
    MOCK_KEYCHAIN_PASSWORD="" \
    sh -c '. "$1" >/dev/null 2>&1 || true; [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]' \
    _ "$SIGNING_ENV"
}

MOCK_KEYCHAIN_PASSWORD="" expect_failure "missing updater passphrase" source_script
expect_no_key_export_after_failure
expect_stale_exports_cleared_after_failure
MOCK_KEYCHAIN_PASSWORD="too-short" expect_failure "weak updater passphrase" source_script
MOCK_KEYCHAIN_PASSWORD="Updater-Key-Password-2026!" source_script >/dev/null

printf '%s\n' 'untrusted comment: minisign secret key' > "$TMP_DIR/private.key"
MOCK_KEYCHAIN_PASSWORD="Updater-Key-Password-2026!" \
  expect_failure "unencrypted updater key" source_script

RSIGN_KEY="$(printf '%s\n%s\n' \
  'untrusted comment: rsign encrypted secret key' \
  'mock-key-material' | openssl base64 -A)"
printf '%s' "$RSIGN_KEY" > "$TMP_DIR/private.key"
env -u TAURI_SIGNING_PRIVATE_KEY_PASSWORD \
  PATH="$TMP_DIR/bin:$PATH" \
  TTREE_UPDATER_KEY_PATH="$TMP_DIR/private.key" \
  MOCK_KEYCHAIN_PASSWORD="Updater-Key-Password-2026!" \
  EXPECTED_KEY="$RSIGN_KEY" \
  sh -c '. "$1" >/dev/null; [ "$TAURI_SIGNING_PRIVATE_KEY" = "$EXPECTED_KEY" ]' \
  _ "$SIGNING_ENV"

echo "signing-env tests passed"
