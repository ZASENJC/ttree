#!/usr/bin/env sh

set -eu

APP_PATH="${1:-}"
EXPECTED_IDENTIFIER="com.samwstu.ttree"
EXPECTED_AUTHORITY="TTREE Open Source Code Signing"
EXPECTED_CERT_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"

if [ -z "$APP_PATH" ] || [ ! -d "$APP_PATH" ]; then
  echo "usage: $0 /path/to/TTREE.app" >&2
  exit 2
fi

if ! VERIFY_OUTPUT="$(codesign --verify --deep --strict --verbose=2 "$APP_PATH" 2>&1)"; then
  printf '%s\n' "$VERIFY_OUTPUT" >&2
  exit 1
fi

DETAILS="$(codesign -d --verbose=4 "$APP_PATH" 2>&1)"
IDENTIFIER="$(printf '%s\n' "$DETAILS" | sed -n 's/^Identifier=//p' | head -n 1)"

if [ "$IDENTIFIER" != "$EXPECTED_IDENTIFIER" ]; then
  echo "[codesign] unexpected identifier: ${IDENTIFIER:-missing}" >&2
  exit 1
fi

if printf '%s\n' "$DETAILS" | grep -q '^Signature=adhoc$'; then
  echo "[codesign] app is unsigned or ad-hoc signed" >&2
  exit 1
fi

if ! printf '%s\n' "$DETAILS" | grep -q '^Signature size='; then
  echo "[codesign] app does not contain a certificate-backed CMS signature" >&2
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
NORMALIZED_REQUIREMENT="$(
  printf '%s\n' "$REQUIREMENT" |
    tr '[:lower:]' '[:upper:]' |
    tr -s '[:space:]' ' ' |
    sed 's/^ //; s/ $//'
)"
EXPECTED_REQUIREMENT="DESIGNATED => IDENTIFIER \"$(printf '%s' "$EXPECTED_IDENTIFIER" | tr '[:lower:]' '[:upper:]')\" AND CERTIFICATE LEAF = H\"$EXPECTED_CERT_SHA1\""

if [ "$NORMALIZED_REQUIREMENT" != "$EXPECTED_REQUIREMENT" ]; then
  echo "[codesign] designated requirement is not the pinned TTREE identity" >&2
  exit 1
fi

echo "[codesign] verified pinned community signature for $EXPECTED_IDENTIFIER"
