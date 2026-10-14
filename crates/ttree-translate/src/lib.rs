//! `ttree-translate` —— 极速网页翻译模块（Google POST + Bing 内存 Token 缓存 + Hedged 主备竞速路由）。

pub mod auto;
pub mod bing;
pub mod google;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Response};
use ttree_core::{validate_language_pair, CancelToken};

pub const MAX_TRANSLATION_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_TRANSLATION_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BING_PAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1800);

/// 共享连接池客户端：内置 1.8s 快速连接超时与 TCP Keepalive，彻底消除不可达节点卡死 15s 的问题。
pub(crate) fn shared_client(timeout: Duration, cookie_store: bool) -> Client {
    static CLIENTS: OnceLock<Mutex<HashMap<(Duration, bool), Client>>> = OnceLock::new();

    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut clients = clients.lock().expect("HTTP client cache lock poisoned");
    clients
        .entry((timeout, cookie_store))
        .or_insert_with(|| {
            Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(timeout)
                .tcp_keepalive(Duration::from_secs(30))
                .pool_idle_timeout(Duration::from_secs(90))
                .cookie_store(cookie_store)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("failed to build reqwest client")
        })
        .clone()
}

pub fn validate_translation_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_TRANSLATION_TEXT_BYTES {
        return Err(format!(
            "待翻译文本超出 {MAX_TRANSLATION_TEXT_BYTES} 字节上限"
        ));
    }
    Ok(())
}

/// 顶层网页翻译入口（支持 `CancelToken` 即时中断与主备竞速）。
pub async fn translate_web(
    text: &str,
    source: &str,
    target: &str,
    cancel: &CancelToken,
) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    validate_translation_text(trimmed)?;
    validate_language_pair(source, target)?;

    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("翻译已取消".to_string()),
        res = auto::translate(trimmed, source, target) => res,
    }
}

pub(crate) async fn read_limited_response(
    response: Response,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("{label}超出 {limit} 字节上限"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取{label}失败: {e}"))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!("{label}超出 {limit} 字节上限"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) async fn read_error_body(response: Response) -> String {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(chunk) => {
                let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len());
                if chunk.len() > remaining {
                    bytes.extend_from_slice(&chunk[..remaining]);
                    truncated = true;
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            Err(e) => return format!("<读取错误响应失败: {e}>"),
        }
    }
    let mut out = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        out.push_str("…[truncated]");
    }
    out
}
