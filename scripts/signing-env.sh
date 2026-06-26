#!/usr/bin/env sh
# 加载 Tauri 更新签名密钥，供 `npm run tauri build` 生成可校验的更新包。
#
# 私钥本身不在仓库内，统一存放在本机固定路径（见 PRIV_KEY_PATH）。
# 私钥口令从 macOS Keychain 读取（account 见 PASS_KEYCHAIN_ACCOUNT），不在脚本里硬编码。
# 用法：source ./scripts/signing-env.sh && npm run tauri build
#   或：  . ./scripts/signing-env.sh && npm run tauri build
#
# 密钥缺失时给出明确提示并退出（非零），避免产出无法被验证的更新包。

PRIV_KEY_PATH="${TTREE_UPDATER_KEY_PATH:-$HOME/.config/ttree-updater/private.key}"
# Keychain 里存放私钥口令的 service / account（与代码内 keychain SERVICE 对齐）。
PASS_KEYCHAIN_SERVICE="${TTREE_UPDATER_PASS_SERVICE:-com.samwstu.ttree}"
PASS_KEYCHAIN_ACCOUNT="${TTREE_UPDATER_PASS_ACCOUNT:-updater-signing-password}"

if [ ! -f "$PRIV_KEY_PATH" ]; then
  echo "[signing-env] 找不到签名私钥: $PRIV_KEY_PATH" >&2
  echo "[signing-env] 请确认已执行密钥生成或恢复步骤。" >&2
  return 1 2>/dev/null || exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY_PATH="$PRIV_KEY_PATH"

# 私钥口令优先级：已显式设置的 TAURI_SIGNING_PRIVATE_KEY_PASSWORD > Keychain > 空。
# 私钥未设口令时，Keychain 查不到，自然回退到空串。
if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
  KC_PASS="$(security find-generic-password -s "$PASS_KEYCHAIN_SERVICE" -a "$PASS_KEYCHAIN_ACCOUNT" -w 2>/dev/null || true)"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$KC_PASS"
fi

echo "[signing-env] 已加载更新签名密钥: $PRIV_KEY_PATH"
