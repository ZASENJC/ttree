#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/bin" "$TMP_DIR/TTREE.app"

cat > "$TMP_DIR/bin/codesign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

case "$*" in
  "--verify --deep --strict --verbose=2 "*)
    exit 0
    ;;
  "-d --verbose=4 "*)
    cat <<DETAILS
Identifier=${MOCK_IDENTIFIER}
Authority=${MOCK_AUTHORITY}
Signature=${MOCK_SIGNATURE}
Info.plist entries=15
TeamIdentifier=not set
Sealed Resources version=2 rules=13 files=1
DETAILS
    ;;
  "-d -r- "*)
    echo "designated => ${MOCK_REQUIREMENT}"
    ;;
  *)
    echo "unexpected codesign invocation: $*" >&2
    exit 2
    ;;
esac
EOF
chmod +x "$TMP_DIR/bin/codesign"

VERIFY="$ROOT_DIR/scripts/verify-macos-community-signature.sh"
EXPECTED_ID="com.samwstu.ttree"
EXPECTED_AUTHORITY="TTREE Open Source Code Signing"
EXPECTED_SHA1="0123456789ABCDEF0123456789ABCDEF01234567"

run_verify() {
  PATH="$TMP_DIR/bin:$PATH" \
    MOCK_IDENTIFIER="${MOCK_IDENTIFIER:-$EXPECTED_ID}" \
    MOCK_AUTHORITY="${MOCK_AUTHORITY:-$EXPECTED_AUTHORITY}" \
    MOCK_SIGNATURE="${MOCK_SIGNATURE:-size=4096}" \
    MOCK_REQUIREMENT="${MOCK_REQUIREMENT:-identifier \"$EXPECTED_ID\" and certificate leaf = H\"$EXPECTED_SHA1\"}" \
    "$VERIFY" "$TMP_DIR/TTREE.app" "$EXPECTED_ID" "$EXPECTED_AUTHORITY" "$EXPECTED_SHA1"
}

expect_failure() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "expected failure: $description" >&2
    exit 1
  fi
}

MOCK_REQUIREMENT="identifier \"$EXPECTED_ID\"" \
  expect_failure "identifier-only requirement" run_verify

MOCK_REQUIREMENT="identifier \"$EXPECTED_ID\" and certificate leaf = H\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"" \
  expect_failure "wrong certificate fingerprint" run_verify

MOCK_SIGNATURE="adhoc" \
  expect_failure "ad-hoc signature" run_verify

MOCK_AUTHORITY="Unexpected Code Signing" \
  expect_failure "wrong signing authority" run_verify

run_verify

echo "verify-macos-community-signature tests passed"
