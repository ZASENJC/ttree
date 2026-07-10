//! macOS Keychain 封装 —— 把敏感凭据（API key）存进系统钥匙串，
//! 避免明文落盘到 settings.json。
//!
//! 用「通用密码」(generic password) 项存储，service 固定为 bundle id，
//! account 区分不同 key。非 macOS 平台返回错误（本项目仅面向 macOS）。

#[cfg(target_os = "macos")]
use security_framework::passwords;
use serde::{Deserialize, Serialize};

/// Keychain 项的 service 名（沿用 bundle identifier）。
pub const SERVICE: &str = "com.samwstu.ttree";

/// OpenAI 翻译配置的 API key 账户名。
pub const ACCOUNT_OPENAI_KEY: &str = "openai.api_key";
/// AI 对话配置的 API key 账户名。
pub const ACCOUNT_CHAT_AI_KEY: &str = "chat_ai.api_key";

/// errSecItemNotFound：Keychain 中找不到该条目（首次使用属正常情况）。
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

const BOUND_SECRET_PREFIX: &str = "ttree-bound-secret-v1:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRecord {
    Legacy(String),
    Bound { endpoint: String, value: String },
}

#[derive(Serialize, Deserialize)]
struct BoundSecretPayload {
    endpoint: String,
    value: String,
}

pub fn encode_bound_secret(endpoint: &str, value: &str) -> Result<String, String> {
    if endpoint.is_empty() || value.is_empty() {
        return Err("接口地址和密钥均不能为空".to_string());
    }
    let payload = serde_json::to_string(&BoundSecretPayload {
        endpoint: endpoint.to_string(),
        value: value.to_string(),
    })
    .map_err(|e| format!("序列化钥匙串记录失败: {e}"))?;
    Ok(format!("{BOUND_SECRET_PREFIX}{payload}"))
}

pub fn decode_secret_record(raw: &str) -> Result<SecretRecord, String> {
    let Some(payload) = raw.strip_prefix(BOUND_SECRET_PREFIX) else {
        return Ok(SecretRecord::Legacy(raw.to_string()));
    };
    let record: BoundSecretPayload =
        serde_json::from_str(payload).map_err(|e| format!("钥匙串记录损坏: {e}"))?;
    if record.endpoint.is_empty() || record.value.is_empty() {
        return Err("钥匙串记录损坏: 接口地址或密钥为空".to_string());
    }
    Ok(SecretRecord::Bound {
        endpoint: record.endpoint,
        value: record.value,
    })
}

pub fn get_secret_record(account: &str) -> Result<Option<SecretRecord>, String> {
    get_secret(account)?
        .map(|raw| decode_secret_record(&raw))
        .transpose()
}

pub fn set_bound_secret(account: &str, endpoint: &str, value: &str) -> Result<(), String> {
    let encoded = encode_bound_secret(endpoint, value)?;
    set_secret(account, &encoded)
}

/// 读取一个凭据。项不存在时返回 `Ok(None)`（首次使用），其余错误原样上抛。
#[cfg(target_os = "macos")]
pub fn get_secret(account: &str) -> Result<Option<String>, String> {
    match passwords::get_generic_password(SERVICE, account) {
        Ok(bytes) => Ok(Some(
            String::from_utf8(bytes).map_err(|e| format!("密钥内容非 UTF-8: {e}"))?,
        )),
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

// ── 非 macOS 占位：本项目目前仅面向 macOS，这里仅作编译护栏 ──
#[cfg(not(target_os = "macos"))]
pub fn get_secret(_account: &str) -> Result<Option<String>, String> {
    Err("此平台不支持 Keychain".to_string())
}
#[cfg(not(target_os = "macos"))]
pub fn set_secret(_account: &str, _value: &str) -> Result<(), String> {
    Err("此平台不支持 Keychain".to_string())
}
#[cfg(not(target_os = "macos"))]
pub fn delete_secret(_account: &str) -> Result<(), String> {
    Err("此平台不支持 Keychain".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_endpoint_bound_secrets() {
        let encoded = encode_bound_secret("https://api.example.com/v1", "top-secret").unwrap();

        assert_eq!(
            decode_secret_record(&encoded).unwrap(),
            SecretRecord::Bound {
                endpoint: "https://api.example.com/v1".to_string(),
                value: "top-secret".to_string(),
            }
        );
        assert!(!encoded.contains("\"api_key\""));
    }

    #[test]
    fn treats_existing_raw_keychain_values_as_legacy() {
        assert_eq!(
            decode_secret_record("sk-existing").unwrap(),
            SecretRecord::Legacy("sk-existing".to_string())
        );
    }

    #[test]
    fn rejects_corrupt_bound_secret_records() {
        assert!(decode_secret_record("ttree-bound-secret-v1:{broken").is_err());
    }
}
