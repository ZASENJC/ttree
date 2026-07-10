#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

bash "$ROOT_DIR/scripts/tests/verify-macos-community-signature.test.sh" >/dev/null
echo "verify-macos-signature compatibility tests passed"
