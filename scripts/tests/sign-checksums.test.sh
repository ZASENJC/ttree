#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin" "$WORK/release"
printf 'fixture-checksum  package.zip\n' > "$WORK/release/SHA256SUMS"
cat > "$WORK/bin/npx" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
input="${!#}"
[[ "$input" == *.txt ]] || { echo 'signer requires a file extension' >&2; exit 1; }
[[ -s "$input" ]]
printf 'dGVzdA==' > "$input.sig"
STUB
cat > "$WORK/bin/minisign" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
while [[ "$#" -gt 0 ]]; do
  if [[ "$1" == -m ]]; then
    shift
    [[ "$(cat "$1")" == 'fixture-checksum  package.zip' ]] || exit 1
  fi
  shift
done
STUB
chmod +x "$WORK/bin/"*
PATH="$WORK/bin:$PATH" \
  TAURI_SIGNING_PRIVATE_KEY='untrusted comment: minisign encrypted secret key fixture' \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD='test-only-password-at-least-16' \
  "$ROOT/scripts/release/sign-checksums.sh" "$WORK/release"
[[ "$(cat "$WORK/release/SHA256SUMS.sig")" == dGVzdA== ]]
[[ "$(cat "$WORK/release/SHA256SUMS")" == 'fixture-checksum  package.zip' ]]
echo 'checksum signer compatibility tests passed'
