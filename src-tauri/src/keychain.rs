//! 凭据安全存储封装 —— 把敏感凭据（API key）存进系统钥匙串，
//! 避免明文落盘到 settings.json。
//!
//! - macOS: 使用 `security-framework` 访问 Keychain (generic password)
//! - Windows/Linux: 使用跨平台 `keyring` crate (Windows Credential Manager / Secret Service)
//!
//! 用「通用密码」(generic password) 项存储，service 固定为 bundle id，
//! account 区分不同 key。

#[cfg(target_os = "macos")]
use security_framework::passwords;

/// Keychain 项的 service 名（沿用 bundle identifier）。
pub const SERVICE: &str = "com.samwstu.ttree";

/// OpenAI 翻译配置的 API key 账户名。
pub const ACCOUNT_OPENAI_KEY: &str = "openai.api_key";
/// AI 对话配置的 API key 账户名。
pub const ACCOUNT_CHAT_AI_KEY: &str = "chat_ai.api_key";

/// errSecItemNotFound：Keychain 中找不到该条目（首次使用属正常情况）。
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

// ── macOS 实现：security-framework ──────────────────────────────────

/// 读取一个凭据。项不存在时返回 `Ok(None)`（首次使用），其余错误原样上抛。
#[cfg(target_os = "macos")]
pub fn get_secret(account: &str) -> Result<Option<String>, String> {
    match passwords::get_generic_password(SERVICE, account) {
        Ok(bytes) => Ok(Some(String::from_utf8(bytes).map_err(|e| {
            format!("密钥内容非 UTF-8: {e}")
        })?)),
        Err(err) => {
            // 项不存在属于正常情况，转成 None
            let code = err.code();
            if code == ERR_SEC_ITEM_NOT_FOUND {
                Ok(None)
            } else {
                Err(format!("读取钥匙串失败 (code {code}): {err}"))
            }
        }
    }
}

/// 写入凭据；若已存在则覆盖。
#[cfg(target_os = "macos")]
pub fn set_secret(account: &str, value: &str) -> Result<(), String> {
    passwords::set_generic_password(SERVICE, account, value.as_bytes())
        .map_err(|e| format!("写入钥匙串失败: {e}"))
}

/// 删除凭据；项不存在视为成功（幂等）。
#[cfg(target_os = "macos")]
pub fn delete_secret(account: &str) -> Result<(), String> {
    match passwords::delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(()),
        Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(err) => Err(format!("删除钥匙串失败 (code {}): {err}", err.code())),
    }
}

// ── 非 macOS 实现：keyring crate (Windows Credential Manager / Linux Secret Service) ──

#[cfg(not(target_os = "macos"))]
pub fn get_secret(account: &str) -> Result<Option<String>, String> {
    use keyring::Entry;
    let entry = Entry::new(SERVICE, account).map_err(|e| format!("创建凭据条目失败: {e}"))?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读取凭据失败: {e}")),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_secret(account: &str, value: &str) -> Result<(), String> {
    use keyring::Entry;
    let entry = Entry::new(SERVICE, account).map_err(|e| format!("创建凭据条目失败: {e}"))?;
    entry
        .set_password(value)
        .map_err(|e| format!("写入凭据失败: {e}"))
}

#[cfg(not(target_os = "macos"))]
pub fn delete_secret(account: &str) -> Result<(), String> {
    use keyring::Entry;
    let entry = Entry::new(SERVICE, account).map_err(|e| format!("创建凭据条目失败: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("删除凭据失败: {e}")),
    }
}
