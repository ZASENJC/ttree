#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release.yml"

require_text() {
  local text="$1"
  if ! grep -Fq "$text" "$WORKFLOW"; then
    echo "release workflow is missing: $text" >&2
    exit 1
  fi
}

reject_text() {
  local text="$1"
  if grep -Fq "$text" "$WORKFLOW"; then
    echo "release workflow must not contain: $text" >&2
    exit 1
  fi
}

require_text 'MACOS_COMMUNITY_CERTIFICATE'
require_text 'MACOS_COMMUNITY_CERTIFICATE_PASSWORD'
require_text 'MACOS_COMMUNITY_SIGNING_IDENTITY'
require_text 'MACOS_COMMUNITY_CERT_SHA1'
require_text './scripts/package-macos-community.sh universal-apple-darwin'
require_text './scripts/verify-macos-community-signature.sh'
require_text 'actions/upload-artifact@v4'
require_text "startsWith(github.ref, 'refs/tags/v')"

reject_text 'APPLE_ID'
reject_text 'APPLE_PASSWORD'
reject_text 'APPLE_TEAM_ID'
reject_text 'xcrun stapler'

echo "release workflow tests passed"
