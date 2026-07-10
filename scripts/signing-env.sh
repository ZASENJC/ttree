#!/usr/bin/env sh
# 加载 Tauri 更新签名密钥，供社区发布打包脚本生成可校验的更新包。
#
# 私钥本身不在仓库内，统一存放在本机固定路径（见 PRIV_KEY_PATH）。
# 私钥口令从 macOS Keychain 读取（account 见 PASS_KEYCHAIN_ACCOUNT），不在脚本里硬编码。
# 用法：source ./scripts/signing-env.sh && ./scripts/package-macos-community.sh
#   或：  . ./scripts/signing-env.sh && ./scripts/package-macos-community.sh
#
# 密钥、加密标记或强口令缺失时给出明确提示并退出（非零），避免产出弱签名更新包。

SIGNING_PASSWORD_OVERRIDE="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD

PRIV_KEY_PATH="${TTREE_UPDATER_KEY_PATH:-$HOME/.config/ttree-updater/private.key}"
# Keychain 里存放私钥口令的 service / account（与代码内 keychain SERVICE 对齐）。
PASS_KEYCHAIN_SERVICE="${TTREE_UPDATER_PASS_SERVICE:-com.samwstu.ttree}"
PASS_KEYCHAIN_ACCOUNT="${TTREE_UPDATER_PASS_ACCOUNT:-updater-signing-password}"

if [ ! -f "$PRIV_KEY_PATH" ]; then
  echo "[signing-env] 找不到签名私钥: $PRIV_KEY_PATH" >&2
  echo "[signing-env] 请确认已执行密钥生成或恢复步骤。" >&2
  unset SIGNING_PASSWORD_OVERRIDE
  return 1 2>/dev/null || exit 1
fi

KEY_VALUE=""
if grep -Eqi '(minisign|rsign) encrypted secret key' "$PRIV_KEY_PATH"; then
  KEY_VALUE="$(openssl base64 -A -in "$PRIV_KEY_PATH")"
elif openssl base64 -d -A -in "$PRIV_KEY_PATH" 2>/dev/null |
  grep -Eqi '(minisign|rsign) encrypted secret key'; then
  KEY_VALUE="$(tr -d '\r\n' < "$PRIV_KEY_PATH")"
else
  echo "[signing-env] 更新签名私钥不是 passphrase 加密的 rsign/minisign key。" >&2
  unset KEY_VALUE SIGNING_PASSWORD_OVERRIDE
  return 1 2>/dev/null || exit 1
fi

# 私钥口令优先级：已显式设置的 TAURI_SIGNING_PRIVATE_KEY_PASSWORD > Keychain。
SIGNING_PASSWORD="$SIGNING_PASSWORD_OVERRIDE"
if [ -z "$SIGNING_PASSWORD" ]; then
  SIGNING_PASSWORD="$(security find-generic-password -s "$PASS_KEYCHAIN_SERVICE" -a "$PASS_KEYCHAIN_ACCOUNT" -w 2>/dev/null || true)"
fi

if [ "${#SIGNING_PASSWORD}" -lt 16 ]; then
  echo "[signing-env] 更新签名私钥口令缺失或少于 16 个字符，拒绝继续。" >&2
  unset KEY_VALUE SIGNING_PASSWORD SIGNING_PASSWORD_OVERRIDE
  return 1 2>/dev/null || exit 1
fi

# 当前 Tauri bundle 流程要求环境变量是“完整 minisign key 文件”的 base64。
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$SIGNING_PASSWORD"
export TAURI_SIGNING_PRIVATE_KEY="$KEY_VALUE"
export TAURI_SIGNING_PRIVATE_KEY_PATH="$PRIV_KEY_PATH"
unset KEY_VALUE SIGNING_PASSWORD SIGNING_PASSWORD_OVERRIDE

echo "[signing-env] 已加载更新签名密钥: $PRIV_KEY_PATH"
