#!/usr/bin/env bash
# Print only the temporary keychain path; never print signing material.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
PINNED_IDENTITY="TTREE Open Source Code Signing"
PINNED_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ttree-signing.XXXXXX")"
KEYCHAIN="$WORK_DIR/signing.keychain-db"
cleanup_failed() { security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true; rm -rf "$WORK_DIR"; }
trap cleanup_failed ERR
umask 077
if [[ -n "${MACOS_COMMUNITY_CERTIFICATE:-}" ]]; then
  "$ROOT_DIR/scripts/release/validate-secrets.sh" certificate >&2
  printf '%s' "$MACOS_COMMUNITY_CERTIFICATE" | openssl base64 -d -A -out "$WORK_DIR/cert.p12"
  CERT_PASSWORD="$MACOS_COMMUNITY_CERTIFICATE_PASSWORD"
else
  cp "$HOME/.config/ttree-code-signing/community-signing.p12" "$WORK_DIR/cert.p12"
  CERT_PASSWORD="$(security find-generic-password -s com.samwstu.ttree -a community-code-signing-password -w)"
fi
KEYCHAIN_PASSWORD="$(openssl rand -hex 24)"
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  echo "::add-mask::$KEYCHAIN_PASSWORD" >&2
fi
echo "[certificate] Creating temporary keychain" >&2
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >&2
security set-keychain-settings -lut 21600 "$KEYCHAIN" >&2
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >&2
echo "[certificate] Importing encrypted P12" >&2
security import "$WORK_DIR/cert.p12" -k "$KEYCHAIN" -P "$CERT_PASSWORD" -T /usr/bin/codesign >/dev/null
echo "[certificate] Configuring private-key access" >&2
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
echo "[certificate] Checking pinned certificate" >&2
security find-certificate -c "$PINNED_IDENTITY" -p "$KEYCHAIN" > "$WORK_DIR/cert.pem"
ACTUAL_SHA1="$(openssl x509 -in "$WORK_DIR/cert.pem" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g' | tr '[:lower:]' '[:upper:]')"
[[ "$ACTUAL_SHA1" == "$PINNED_SHA1" ]] || { echo "Wrong community certificate" >&2; false; }
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  echo "[certificate] Trusting the pinned identity on disposable CI runner" >&2
  sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain "$WORK_DIR/cert.pem" >&2
fi
rm -f "$WORK_DIR/cert.p12" "$WORK_DIR/cert.pem"
printf '%s\n' "$KEYCHAIN"
