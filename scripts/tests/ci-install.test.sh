#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
INSTALL_SCRIPT="$ROOT_DIR/scripts/ci-install.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if [[ "$(jq -r '.scripts["ci:install"] // empty' "$ROOT_DIR/package.json")" != "bash scripts/ci-install.sh" ]]; then
  echo "package.json must expose ci:install through scripts/ci-install.sh" >&2
  exit 1
fi

if jq -e 'has("allowScripts")' "$ROOT_DIR/package.json" >/dev/null; then
  echo "package.json must not rely on npm-ignored allowScripts" >&2
  exit 1
fi

mkdir -p "$TMP_DIR/bin"
cat > "$TMP_DIR/bin/npm" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$MOCK_NPM_LOG"
EOF
chmod +x "$TMP_DIR/bin/npm"

MOCK_NPM_LOG="$TMP_DIR/npm.log" PATH="$TMP_DIR/bin:$PATH" bash "$INSTALL_SCRIPT"

EXPECTED="$(printf '%s\n%s\n' 'ci --ignore-scripts' 'rebuild esbuild fsevents')"
ACTUAL="$(cat "$TMP_DIR/npm.log")"
if [[ "$ACTUAL" != "$EXPECTED" ]]; then
  echo "ci-install.sh ran unexpected npm commands" >&2
  exit 1
fi

echo "ci-install tests passed"
