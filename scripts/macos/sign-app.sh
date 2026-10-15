#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
APP_PATH="${1:?app bundle required}"
KEYCHAIN="${2:?temporary signing keychain required}"
REQUIREMENT='=designated => identifier "com.samwstu.ttree" and certificate leaf = H"B8D254561BB6CAD78AA71FA18D62A32D10F50C08"'
codesign --force --options runtime --timestamp=none --sign B8D254561BB6CAD78AA71FA18D62A32D10F50C08 \
  --identifier com.samwstu.ttree --requirements "$REQUIREMENT" --keychain "$KEYCHAIN" "$APP_PATH"
"$ROOT_DIR/scripts/macos/verify-signature.sh" "$APP_PATH"
