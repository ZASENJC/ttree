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
    exit "${MOCK_VERIFY_EXIT:-0}"
    ;;
  "-d --verbose=4 "*)
    cat <<DETAILS
Identifier=${MOCK_IDENTIFIER}
Authority=${MOCK_AUTHORITY}
${MOCK_SIGNATURE}
${MOCK_INFO_PLIST}
TeamIdentifier=not set
${MOCK_SEALED_RESOURCES}
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
EXPECTED_SHA1="B8D254561BB6CAD78AA71FA18D62A32D10F50C08"

run_verify() {
  PATH="$TMP_DIR/bin:$PATH" \
    MOCK_IDENTIFIER="${MOCK_IDENTIFIER-$EXPECTED_ID}" \
    MOCK_AUTHORITY="${MOCK_AUTHORITY-$EXPECTED_AUTHORITY}" \
    MOCK_SIGNATURE="${MOCK_SIGNATURE-Signature size=4096}" \
    MOCK_INFO_PLIST="${MOCK_INFO_PLIST-Info.plist entries=15}" \
    MOCK_SEALED_RESOURCES="${MOCK_SEALED_RESOURCES-Sealed Resources version=2 rules=13 files=1}" \
    MOCK_REQUIREMENT="${MOCK_REQUIREMENT-identifier \"$EXPECTED_ID\" and certificate leaf = H\"$EXPECTED_SHA1\"}" \
    MOCK_VERIFY_EXIT="${MOCK_VERIFY_EXIT-0}" \
    "$VERIFY" "$TMP_DIR/TTREE.app"
}

expect_failure() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "expected failure: $description" >&2
    exit 1
  fi
}

MOCK_IDENTIFIER="com.example.impostor" expect_failure "wrong bundle identifier" run_verify
MOCK_REQUIREMENT="identifier \"$EXPECTED_ID\"" expect_failure "identifier-only requirement" run_verify
MOCK_REQUIREMENT="identifier \"$EXPECTED_ID\" and certificate leaf = H\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"" \
  expect_failure "wrong certificate fingerprint" run_verify
MOCK_REQUIREMENT="cdhash H\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"" \
  expect_failure "CDHash requirement" run_verify
MOCK_SIGNATURE="Signature=adhoc" expect_failure "ad-hoc signature" run_verify
MOCK_AUTHORITY="Unexpected Code Signing" expect_failure "wrong signing authority" run_verify
MOCK_INFO_PLIST="" expect_failure "unbound Info.plist" run_verify
MOCK_SEALED_RESOURCES="" expect_failure "unsealed resources" run_verify
MOCK_VERIFY_EXIT=1 expect_failure "codesign verification failure" run_verify

run_verify >/dev/null
echo "verify-macos-community-signature tests passed"
