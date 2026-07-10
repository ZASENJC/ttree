#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
VERIFY="$ROOT_DIR/scripts/verify-macos-release-assets.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

RELEASE_DIR="$TMP_DIR/release"
MOCK_APP="$TMP_DIR/source/TTREE.app"
VERSION="0.1.0"
TAG="v$VERSION"
REPOSITORY="ZASENJC/ttree"
DMG_NAME="TTREE_${VERSION}_universal.dmg"
ARCHIVE_NAME="TTREE_universal.app.tar.gz"
SIGNATURE="bW9jay1taW5pc2lnbi1zaWduYXR1cmU="

mkdir -p "$RELEASE_DIR" "$MOCK_APP/Contents/MacOS" "$TMP_DIR/bin"
printf '#!/usr/bin/env sh\n' > "$MOCK_APP/Contents/MacOS/ttree"
chmod +x "$MOCK_APP/Contents/MacOS/ttree"
tar -czf "$RELEASE_DIR/$ARCHIVE_NAME" -C "$TMP_DIR/source" TTREE.app
printf '%s\n' 'mock dmg' > "$RELEASE_DIR/$DMG_NAME"
printf '%s\n' "$SIGNATURE" > "$RELEASE_DIR/$ARCHIVE_NAME.sig"

jq -n \
  --arg version "$VERSION" \
  --arg signature "$SIGNATURE" \
  --arg url "https://github.com/$REPOSITORY/releases/download/$TAG/$ARCHIVE_NAME" \
  '{version: $version, notes: "test", pub_date: "2026-07-10T00:00:00.000Z", platforms: {
    "darwin-aarch64": {signature: $signature, url: $url},
    "darwin-x86_64": {signature: $signature, url: $url},
    "darwin-aarch64-app": {signature: $signature, url: $url},
    "darwin-x86_64-app": {signature: $signature, url: $url}
  }}' > "$RELEASE_DIR/latest.json"

cat > "$TMP_DIR/bin/codesign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  "--verify --deep --strict --verbose=2 "*) exit 0 ;;
  "-d --verbose=4 "*)
    cat <<DETAILS
Identifier=com.samwstu.ttree
Authority=TTREE Open Source Code Signing
Signature size=4096
Info.plist entries=15
TeamIdentifier=not set
Sealed Resources version=2 rules=13 files=1
DETAILS
    ;;
  "-d -r- "*)
    echo "Executable=$3/Contents/MacOS/ttree"
    echo 'designated => identifier "com.samwstu.ttree" and certificate leaf = H"B8D254561BB6CAD78AA71FA18D62A32D10F50C08"'
    ;;
  *) exit 2 ;;
esac
EOF

cat > "$TMP_DIR/bin/hdiutil" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == "attach" ]]; then
  mountpoint=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "-mountpoint" ]]; then
      mountpoint="$2"
      break
    fi
    shift
  done
  mkdir -p "$mountpoint/TTREE.app/Contents/MacOS"
  printf '#!/usr/bin/env sh\n' > "$mountpoint/TTREE.app/Contents/MacOS/ttree"
  chmod +x "$mountpoint/TTREE.app/Contents/MacOS/ttree"
  exit 0
fi
if [[ "$1" == "detach" ]]; then
  exit 0
fi
exit 2
EOF

cat > "$TMP_DIR/bin/minisign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${MOCK_MINISIGN_FAIL:-0}" == "0" ]] || exit 1
[[ "$*" == *" -m "* && "$*" == *" -x "* && "$*" == *" -p "* ]]
EOF
chmod +x "$TMP_DIR/bin/codesign" "$TMP_DIR/bin/hdiutil" "$TMP_DIR/bin/minisign"

run_verify() {
  PATH="$TMP_DIR/bin:$PATH" MOCK_MINISIGN_FAIL="${MOCK_MINISIGN_FAIL:-0}" \
    "$VERIFY" "$RELEASE_DIR" "$TAG" "$REPOSITORY" "$VERSION"
}

expect_failure() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "expected failure: $description" >&2
    exit 1
  fi
}

run_verify >/dev/null

MOCK_MINISIGN_FAIL=1 expect_failure "invalid updater signature" run_verify

cp "$RELEASE_DIR/latest.json" "$TMP_DIR/latest.json"
jq '.version = "9.9.9"' "$TMP_DIR/latest.json" > "$RELEASE_DIR/latest.json"
expect_failure "manifest version mismatch" run_verify
cp "$TMP_DIR/latest.json" "$RELEASE_DIR/latest.json"

mv "$RELEASE_DIR/$DMG_NAME" "$TMP_DIR/$DMG_NAME"
expect_failure "missing DMG" run_verify
mv "$TMP_DIR/$DMG_NAME" "$RELEASE_DIR/$DMG_NAME"

echo "verify-macos-release-assets tests passed"
