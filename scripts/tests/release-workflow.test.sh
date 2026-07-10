#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
WORKFLOW="$ROOT_DIR/.github/workflows/release.yml"
PINNED_CERT_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"

fail() {
  echo "release workflow contract failed: $*" >&2
  exit 1
}

require_text() {
  local text="$1"
  grep -Fq -- "$text" "$WORKFLOW" || fail "missing: $text"
}

reject_text() {
  local text="$1"
  if grep -Fq -- "$text" "$WORKFLOW"; then
    fail "forbidden text present: $text"
  fi
}

line_number() {
  local text="$1"
  grep -Fnm1 -- "$text" "$WORKFLOW" | cut -d: -f1
}

assert_before() {
  local first="$1"
  local second="$2"
  local first_line second_line
  first_line="$(line_number "$first")"
  second_line="$(line_number "$second")"
  [[ -n "$first_line" && -n "$second_line" && "$first_line" -lt "$second_line" ]] ||
    fail "expected '$first' before '$second'"
}

require_text "EXPECTED_MACOS_COMMUNITY_CERT_SHA1: $PINNED_CERT_SHA1"
require_text 'MACOS_COMMUNITY_CERTIFICATE'
require_text 'MACOS_COMMUNITY_CERTIFICATE_PASSWORD'
require_text 'TAURI_SIGNING_PRIVATE_KEY'
require_text 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'
require_text './scripts/validate-release-secrets.sh'
require_text './scripts/validate-release-secrets.sh certificate'
require_text './scripts/validate-release-secrets.sh updater'

require_text 'package_version="$(jq -r '\''.version'\'' package.json)"'
require_text 'tauri_version="$(jq -r '\''.version'\'' src-tauri/tauri.conf.json)"'
require_text 'expected_tag="v$package_version"'
require_text 'Tag $GITHUB_REF_NAME does not match package version $package_version'
require_text 'package.json version $package_version does not match tauri.conf.json version $tauri_version'

require_text 'npm run ci:install'
reject_text 'run: npm ci'
require_text 'npm run check'
require_text 'npm test'
require_text 'cargo check --manifest-path src-tauri/Cargo.toml'
require_text 'cargo test --manifest-path src-tauri/Cargo.toml'
require_text 'cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings'
require_text './scripts/package-macos-community.sh universal-apple-darwin'
require_text './scripts/package-macos-community.sh universal-apple-darwin "$GITHUB_RUN_NUMBER" --build-only'
require_text './scripts/package-macos-community.sh universal-apple-darwin "$GITHUB_RUN_NUMBER" --package-only'
require_text './scripts/verify-macos-community-signature.sh'
require_text './scripts/verify-macos-release-assets.sh'
require_text 'shasum -a 256 --check SHA256SUMS'
require_text 'trap cleanup_import_files EXIT'

require_text 'workflow_dispatch:'
require_text "if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')"
require_text 'gh release create "$RELEASE_TAG"'
require_text 'gh release delete "$RELEASE_TAG"'
require_text 'existing_tag="$RELEASE_TAG"'
require_text 'gh release delete "$existing_tag"'
require_text '--json isDraft'
require_text 'Release $RELEASE_TAG is already published'
require_text '--yes'
require_text '--draft'
require_text 'gh release download "$RELEASE_TAG"'
require_text 'gh release edit "$RELEASE_TAG"'
require_text '--draft=false'
require_text 'id: draft-release'
require_text 'id: publish-release'
require_text "steps.draft-release.outcome == 'success'"
require_text "steps.publish-release.outcome != 'success'"
require_text 'TTREE_${APP_VERSION}_universal.dmg'
require_text 'TTREE_universal.app.tar.gz'
require_text 'TTREE_universal.app.tar.gz.sig'
require_text 'latest.json'

reject_text 'APPLE_ID'
reject_text 'APPLE_PASSWORD'
reject_text 'APPLE_TEAM_ID'
reject_text 'APPLE_CERTIFICATE'
reject_text 'xcrun stapler'
reject_text 'tauri-apps/tauri-action@'
reject_text 'secrets.MACOS_COMMUNITY_CERT_SHA1'
reject_text '--cleanup-tag'

verify_job_env="$(awk '
  /^    env:/ { in_env = 1; next }
  /^    steps:/ { exit }
  in_env { print }
' "$WORKFLOW")"
if grep -Fq 'secrets.' <<< "$verify_job_env"; then
  fail "verify job-level environment must not expose secrets to every step"
fi

action_count=0
while IFS= read -r action; do
  action_count=$((action_count + 1))
  if [[ ! "$action" =~ @[0-9a-f]{40}$ ]]; then
    fail "third-party action is not pinned to a full commit SHA: $action"
  fi
done < <(sed -nE 's/^[[:space:]]*(-[[:space:]]*)?uses:[[:space:]]*([^[:space:]#]+).*/\2/p' "$WORKFLOW")
[[ "$action_count" -ge 5 ]] || fail "expected pinned checkout/setup/cache/upload/download actions"

assert_before './scripts/verify-macos-release-assets.sh' 'actions/upload-artifact@'
assert_before 'brew list minisign' 'MACOS_COMMUNITY_CERTIFICATE: ${{ secrets.MACOS_COMMUNITY_CERTIFICATE }}'
assert_before '--build-only' 'MACOS_COMMUNITY_CERTIFICATE: ${{ secrets.MACOS_COMMUNITY_CERTIFICATE }}'
assert_before 'MACOS_COMMUNITY_CERTIFICATE: ${{ secrets.MACOS_COMMUNITY_CERTIFICATE }}' '--package-only'
assert_before 'Clean up signing keychain' 'Verify final app signature'
assert_before 'Clean up signing keychain' 'actions/upload-artifact@'
assert_before 'gh release create "$RELEASE_TAG"' 'gh release download "$RELEASE_TAG"'
assert_before 'gh release download "$RELEASE_TAG"' 'gh release edit "$RELEASE_TAG"'
assert_before 'gh release create "$RELEASE_TAG"' 'gh release delete "$RELEASE_TAG"'

echo "release workflow tests passed"
