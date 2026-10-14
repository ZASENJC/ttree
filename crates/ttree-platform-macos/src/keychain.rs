//! macOS Keychain 封装 —— 兼容 `com.samwstu.ttree` 既有凭据格式。

#[cfg(target_os = "macos")]
use security_framework::passwords;
use serde::{Deserialize, Serialize};

pub const SERVICE: &str = "com.samwstu.ttree";
pub const ACCOUNT_OPENAI_KEY: &str = "openai.api_key";
pub const ACCOUNT_CHAT_AI_KEY: &str = "chat_ai.api_key";

const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const BOUND_SECRET_PREFIX: &str = "ttree-bound-secret-v1:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRecord {
    Legacy(String),
    Bound { endpoint: String, value: String },
}

impl SecretRecord {
    pub fn value(&self) -> &str {
        match self {
            Self::Legacy(v) => v,
            Self::Bound { value, .. } => value,
        }
    }
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

#[cfg(target_os = "macos")]
pub fn get_secret(account: &str) -> Result<Option<String>, String> {
    match passwords::get_generic_password(SERVICE, account) {
        Ok(bytes) => Ok(Some(
            String::from_utf8(bytes).map_err(|e| format!("密钥内容非 UTF-8: {e}"))?,
        )),
        Err(err) => {
            if err.code() == ERR_SEC_ITEM_NOT_FOUND {
                Ok(None)
            } else {
                Err(format!("读取钥匙串失败 (code {}): {err}", err.code()))
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_secret(account: &str, value: &str) -> Result<(), String> {
    passwords::set_generic_password(SERVICE, account, value.as_bytes())
        .map_err(|e| format!("写入钥匙串失败: {e}"))
}

#[cfg(target_os = "macos")]
pub fn delete_secret(account: &str) -> Result<(), String> {
    match passwords::delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(()),
        Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(err) => Err(format!("删除钥匙串失败 (code {}): {err}", err.code())),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_secret(_account: &str) -> Result<Option<String>, String> {
    Ok(None)
}
#[cfg(not(target_os = "macos"))]
pub fn set_secret(_account: &str, _value: &str) -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn delete_secret(_account: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bound_secret() {
        let encoded = encode_bound_secret("https://api.openai.com/v1", "sk-test").unwrap();
        let decoded = decode_secret_record(&encoded).unwrap();
        assert_eq!(decoded.value(), "sk-test");
    }
}
