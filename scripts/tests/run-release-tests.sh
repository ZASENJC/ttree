#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
python3 "$SCRIPT_DIR/release_test.py"
TESTS=(release-secrets.test.sh signing-env.test.sh verify-macos-community-signature.test.sh verify-macos-release-assets.test.sh)

for test_file in "${TESTS[@]}"; do
  bash "$SCRIPT_DIR/$test_file"
done
