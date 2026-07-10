#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS=(
  ci-install.test.sh
  package-macos-community.test.sh
  project-invariants.test.sh
  release-workflow.test.sh
  release-secrets.test.sh
  signing-env.test.sh
  verify-macos-community-signature.test.sh
  verify-macos-release-assets.test.sh
  verify-macos-signature.test.sh
)

for test_file in "${TESTS[@]}"; do
  bash "$SCRIPT_DIR/$test_file"
done
