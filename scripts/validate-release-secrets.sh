#!/usr/bin/env bash

set -euo pipefail

MIN_PASSWORD_LENGTH=16
MODE="${1:-all}"

fail() {
  echo "[release-secrets] $*" >&2
  exit 1
}

require_value() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    fail "missing required secret: $name"
  fi
}

require_strong_password() {
  local name="$1"
  local value="${!name:-}"
  if [[ "${#value}" -lt "$MIN_PASSWORD_LENGTH" ]]; then
    fail "$name must contain at least $MIN_PASSWORD_LENGTH characters"
  fi
}

contains_encrypted_key_marker() {
  grep -Eqi '(minisign|rsign) encrypted secret key'
}

updater_key_is_encrypted() {
  local key="${TAURI_SIGNING_PRIVATE_KEY:-}"

  if printf '%s' "$key" | contains_encrypted_key_marker; then
    return 0
  fi

  if printf '%s' "$key" | openssl base64 -d -A 2>/dev/null | contains_encrypted_key_marker; then
    return 0
  fi

  return 1
}

case "$MODE" in
  certificate)
    require_value MACOS_COMMUNITY_CERTIFICATE
    require_value MACOS_COMMUNITY_CERTIFICATE_PASSWORD
    require_strong_password MACOS_COMMUNITY_CERTIFICATE_PASSWORD
    ;;
  updater)
    require_value TAURI_SIGNING_PRIVATE_KEY
    require_value TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    require_strong_password TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    if ! updater_key_is_encrypted; then
      fail "TAURI_SIGNING_PRIVATE_KEY must be a passphrase-encrypted rsign/minisign private key"
    fi
    ;;
  all)
    "$0" certificate
    "$0" updater
    ;;
  *)
    fail "usage: $0 [certificate|updater|all]"
    ;;
esac

echo "[release-secrets] required encrypted signing material is present"
