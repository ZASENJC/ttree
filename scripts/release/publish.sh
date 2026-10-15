#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"
: "${APP_VERSION:?}" "${RELEASE_TAG:?}" "${PRERELEASE:?}" "${GITHUB_SHA:?}" "${GITHUB_REPOSITORY:?}"
RELEASE_DIR="$ROOT_DIR/dist/release"
NOTES="$RUNNER_TEMP/release-notes.md"
DOWNLOADED="$RUNNER_TEMP/published-assets"
DRAFT_CREATED=0
PUBLISHED=0
cleanup() {
  if [[ "$DRAFT_CREATED" == 1 && "$PUBLISHED" == 0 ]]; then
    # A publish request may succeed despite a lost response. Remove drafts only.
    if [[ "$(gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --json isDraft --jq '.isDraft' 2>/dev/null || true)" == true ]]; then
      gh release delete "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --yes || true
    fi
  fi
}
trap cleanup EXIT
python3 scripts/release/verify-assets.py "$RELEASE_DIR" "$APP_VERSION"
python3 scripts/release/checksums.py "$RELEASE_DIR" --check SHA256SUMS
scripts/release/verify-checksums-signature.sh "$RELEASE_DIR"
if gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
  echo "Release $RELEASE_TAG already exists; refusing to replace it" >&2
  exit 1
fi
{
  printf '# TTREE %s\n\n' "$APP_VERSION"
  if [[ "$PRERELEASE" == true ]]; then printf '> main 自动构建预发布版。正式使用请下载 Latest 稳定版。\n\n'; fi
  printf '## 下载\n\n| 平台 | 文件 |\n| --- | --- |\n'
  printf '| macOS · Apple 芯片与 Intel | `TTREE_%s_universal.dmg` |\n' "$APP_VERSION"
  printf '| Windows · x64 便携版 | `TTREE_%s_windows_x64.zip` |\n\n' "$APP_VERSION"
  cat scripts/release/installation-notes.md
  printf '\n## 本次更新\n\n'
  gh api "repos/$GITHUB_REPOSITORY/releases/generate-notes" -f tag_name="$RELEASE_TAG" -f target_commitish="$GITHUB_SHA" --jq '.body'
  PREVIOUS="$(git describe --tags --match 'v[0-9]*' --exclude '*-dev.*' --abbrev=0 "$GITHUB_SHA^" 2>/dev/null || true)"
  if [[ -n "$PREVIOUS" ]]; then
    printf '\n### 提交记录\n\n'
    git log --no-merges --format='- %s (%h)' "$PREVIOUS..$GITHUB_SHA" -- . ':!Cargo.lock'
  fi
  printf '\n构建提交：`%s`。附件附有 `SHA256SUMS` 校验值。\n' "$GITHUB_SHA"
} > "$NOTES"
FLAGS=(--draft --title "TTREE $RELEASE_TAG" --notes-file "$NOTES" --target "$GITHUB_SHA")
if [[ "$PRERELEASE" == true ]]; then FLAGS+=(--prerelease --latest=false); else FLAGS+=(--verify-tag --latest); fi
gh release create "$RELEASE_TAG" "$RELEASE_DIR"/* --repo "$GITHUB_REPOSITORY" "${FLAGS[@]}"
DRAFT_CREATED=1
mkdir -p "$DOWNLOADED"
gh release download "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --dir "$DOWNLOADED"
python3 scripts/release/verify-assets.py "$DOWNLOADED" "$APP_VERSION"
python3 scripts/release/checksums.py "$DOWNLOADED" --check SHA256SUMS
scripts/release/verify-checksums-signature.sh "$DOWNLOADED"
VERIFY_MACOS_CONTAINERS=0 scripts/release/verify-macos-assets.sh "$DOWNLOADED" "$RELEASE_TAG" "$GITHUB_REPOSITORY" "$APP_VERSION"
if [[ "$PRERELEASE" == true ]]; then
  gh release edit "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --draft=false --latest=false
else
  gh release edit "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --draft=false --latest
fi
PUBLISHED=1
