#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
VALIDATE="$ROOT_DIR/scripts/validate-release-secrets.sh"
ENCRYPTED_UPDATER_KEY="$(printf '%s\n%s\n' \
  'untrusted comment: rsign encrypted secret key' \
  'mock-key-material' | openssl base64 -A)"

run_validate() {
  env \
    MACOS_COMMUNITY_CERTIFICATE="${MACOS_COMMUNITY_CERTIFICATE-test-certificate}" \
    MACOS_COMMUNITY_CERTIFICATE_PASSWORD="${MACOS_COMMUNITY_CERTIFICATE_PASSWORD-Community-Cert-Password-2026!}" \
    TAURI_SIGNING_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY-$ENCRYPTED_UPDATER_KEY}" \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-Updater-Key-Password-2026!}" \
    "$VALIDATE"
}

expect_failure() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "expected failure: $description" >&2
    exit 1
  fi
}

MACOS_COMMUNITY_CERTIFICATE="" expect_failure "missing community certificate" run_validate
MACOS_COMMUNITY_CERTIFICATE_PASSWORD="short" expect_failure "weak certificate password" run_validate
TAURI_SIGNING_PRIVATE_KEY="" expect_failure "missing updater private key" run_validate
TAURI_SIGNING_PRIVATE_KEY="$(printf '%s' 'unencrypted updater key' | openssl base64 -A)" \
  expect_failure "unencrypted updater private key" run_validate
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="short" expect_failure "weak updater key password" run_validate

run_validate >/dev/null

TAURI_SIGNING_PRIVATE_KEY="$(printf '%s\n%s\n' \
  'untrusted comment: minisign encrypted secret key' \
  'legacy-key-material')" run_validate >/dev/null
echo "release secret tests passed"
