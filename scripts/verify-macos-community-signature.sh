#!/usr/bin/env sh

set -eu

APP_PATH="${1:-}"
EXPECTED_IDENTIFIER="${2:-com.samwstu.ttree}"
EXPECTED_AUTHORITY="${3:-TTREE Open Source Code Signing}"
EXPECTED_CERT_SHA1="$(printf '%s' "${4:-}" | tr -d ':' | tr '[:lower:]' '[:upper:]')"

if [ -z "$APP_PATH" ] || [ ! -d "$APP_PATH" ] || [ -z "$EXPECTED_CERT_SHA1" ]; then
  echo "usage: $0 /path/to/TTREE.app [expected-bundle-identifier] [expected-authority] expected-cert-sha1" >&2
  exit 2
fi

if [ "${#EXPECTED_CERT_SHA1}" -ne 40 ]; then
  echo "[codesign] expected certificate SHA-1 must contain exactly 40 hexadecimal digits" >&2
  exit 2
fi

case "$EXPECTED_CERT_SHA1" in
  *[!0-9A-F]*)
    echo "[codesign] expected certificate SHA-1 must contain exactly 40 hexadecimal digits" >&2
    exit 2
    ;;
esac

if ! VERIFY_OUTPUT="$(codesign --verify --deep --strict --verbose=2 "$APP_PATH" 2>&1)"; then
  printf '%s\n' "$VERIFY_OUTPUT" >&2
  exit 1
fi

DETAILS="$(codesign -d --verbose=4 "$APP_PATH" 2>&1)"
IDENTIFIER="$(printf '%s\n' "$DETAILS" | sed -n 's/^Identifier=//p' | head -n 1)"
SIGNATURE="$(printf '%s\n' "$DETAILS" | sed -n 's/^Signature=//p' | head -n 1)"

if [ "$IDENTIFIER" != "$EXPECTED_IDENTIFIER" ]; then
  echo "[codesign] unexpected identifier: ${IDENTIFIER:-missing}" >&2
  exit 1
fi

if [ -z "$SIGNATURE" ] || [ "$SIGNATURE" = "adhoc" ]; then
  echo "[codesign] app is unsigned or ad-hoc signed" >&2
  exit 1
fi

if ! printf '%s\n' "$DETAILS" | grep -Fqx "Authority=$EXPECTED_AUTHORITY"; then
  echo "[codesign] unexpected signing authority" >&2
  exit 1
fi

if ! printf '%s\n' "$DETAILS" | grep -q '^Info.plist entries='; then
  echo "[codesign] Info.plist is not bound into the signature" >&2
  exit 1
fi

if ! printf '%s\n' "$DETAILS" | grep -q '^Sealed Resources version='; then
  echo "[codesign] bundle resources are not sealed" >&2
  exit 1
fi

REQUIREMENT="$(codesign -d -r- "$APP_PATH" 2>&1)"
REQUIREMENT_UPPER="$(printf '%s\n' "$REQUIREMENT" | tr '[:lower:]' '[:upper:]')"
EXPECTED_ID_UPPER="$(printf '%s' "$EXPECTED_IDENTIFIER" | tr '[:lower:]' '[:upper:]')"

if printf '%s\n' "$REQUIREMENT_UPPER" | grep -q 'DESIGNATED => CDHASH'; then
  echo "[codesign] designated requirement is tied to a changing CDHash" >&2
  exit 1
fi

if ! printf '%s\n' "$REQUIREMENT_UPPER" | grep -Fq "IDENTIFIER \"$EXPECTED_ID_UPPER\""; then
  echo "[codesign] designated requirement does not bind the bundle identifier" >&2
  exit 1
fi

if ! printf '%s\n' "$REQUIREMENT_UPPER" | grep -Fq "CERTIFICATE LEAF = H\"$EXPECTED_CERT_SHA1\""; then
  echo "[codesign] designated requirement does not bind the expected certificate" >&2
  exit 1
fi

echo "[codesign] verified community signature for $EXPECTED_IDENTIFIER ($EXPECTED_AUTHORITY)"
