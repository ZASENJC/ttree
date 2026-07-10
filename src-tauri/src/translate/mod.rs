//! 翻译引擎：统一分发 + 各引擎实现。

pub mod bing;
pub mod google;
pub mod openai;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};

/// 复用连接池的 HTTP 客户端。
///
/// `cookie_store=true` 时返回一个独立实例（带 cookie jar），否则返回默认实例。
/// 相同参数多次调用复用同一客户端，避免反复握手建连。
pub(crate) fn shared_client(timeout: Duration, cookie_store: bool) -> Client {
    static CLIENTS: OnceLock<Mutex<HashMap<(Duration, bool), Client>>> = OnceLock::new();

    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut clients = clients.lock().expect("HTTP 客户端缓存锁已损坏");
    clients
        .entry((timeout, cookie_store))
        .or_insert_with(|| {
            Client::builder()
                .timeout(timeout)
                .cookie_store(cookie_store)
                // 禁止跟随重定向：OpenAI 兼容接口为单跳 POST，禁止 3xx 可避免
                // 服务器把带 api_key 的请求重定向到内网/其它地址（防 SSRF 二跳）。
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("构建 HTTP 客户端失败")
        })
        .clone()
}

pub(crate) const MAX_TRANSLATION_TEXT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TRANSLATION_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_BING_PAGE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
pub(crate) const MAX_SSE_FRAME_BYTES: usize = 256 * 1024;
pub(crate) const MAX_STREAM_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

pub fn validate_translation_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_TRANSLATION_TEXT_BYTES {
        return Err(format!(
            "待翻译文本超出 {} 字节上限",
            MAX_TRANSLATION_TEXT_BYTES
        ));
    }
    Ok(())
}

const SUPPORTED_SOURCE_LANGUAGES: &[&str] = &[
    "auto", "zh-CN", "zh-TW", "en", "ja", "ko", "fr", "de", "es", "ru",
];
const SUPPORTED_TARGET_LANGUAGES: &[&str] =
    &["zh-CN", "zh-TW", "en", "ja", "ko", "fr", "de", "es", "ru"];

pub fn validate_language_pair(source: &str, target: &str) -> Result<(), String> {
    if !SUPPORTED_SOURCE_LANGUAGES.contains(&source) {
        return Err(format!("不支持的源语言代码: {source}"));
    }
    if !SUPPORTED_TARGET_LANGUAGES.contains(&target) {
        return Err(format!("不支持的目标语言代码: {target}"));
    }
    Ok(())
}

struct LimitedBody<'a> {
    bytes: Vec<u8>,
    limit: usize,
    label: &'a str,
}

impl<'a> LimitedBody<'a> {
    fn new(limit: usize, label: &'a str) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            label,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), String> {
        let new_len = self
            .bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| format!("{}大小溢出", self.label))?;
        if new_len > self.limit {
            return Err(format!("{}超出 {} 字节上限", self.label, self.limit));
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
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
    let mut body = LimitedBody::new(limit, label);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取{label}失败: {e}"))?;
        body.push(&chunk)?;
    }
    Ok(body.finish())
}

struct ErrorBody {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl ErrorBody {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
        }
    }

    /// 返回 true 表示已达到上限，调用方应停止读取响应体。
    fn push(&mut self, chunk: &[u8]) -> bool {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if chunk.len() > remaining {
            self.bytes.extend_from_slice(&chunk[..remaining]);
            self.truncated = true;
            true
        } else {
            self.bytes.extend_from_slice(chunk);
            false
        }
    }

    fn finish(self) -> String {
        let mut body = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            body.push_str("…[truncated]");
        }
        body
    }
}

pub(crate) async fn read_error_body(response: Response) -> String {
    let mut body = ErrorBody::new(MAX_ERROR_BODY_BYTES);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(chunk) if body.push(&chunk) => break,
            Ok(_) => {}
            Err(error) => return format!("<读取错误响应失败: {error}>"),
        }
    }
    body.finish()
}

/// 翻译引擎类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Openai,
    Google,
    Bing,
}

/// 翻译请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateRequest {
    /// 前端生成的单调请求标识，用于隔离并发/过期流事件。
    pub request_id: u64,
    pub text: String,
    pub engine: Engine,
    /// 源语言代码，"auto" 表示自动检测。
    pub source: String,
    /// 目标语言代码。
    pub target: String,
}

/// AI 对话消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 允许转发的对话角色（白名单）。其余角色一律拒绝，避免任意 system 注入。
const ALLOWED_ROLES: &[&str] = &["user", "assistant", "system"];
/// 单条消息内容长度上限（字节），防御磁盘占用与上游 payload 滥用。
const MAX_CONTENT_BYTES: usize = 64 * 1024;
/// 单次对话请求的消息条数上限。
const MAX_MESSAGES: usize = 200;

/// 校验一组对话消息：角色白名单、单条长度、条数上限。
/// 在 Tauri 命令边界调用，确保 IPC 来的不可信载荷不会原样落盘或转发上游。
pub fn validate_messages(msgs: &[ChatMessage]) -> Result<(), String> {
    if msgs.len() > MAX_MESSAGES {
        return Err(format!("消息数超出上限 {MAX_MESSAGES} 条"));
    }
    for m in msgs {
        if !ALLOWED_ROLES.contains(&m.role.as_str()) {
            return Err(format!("非法消息角色: {}", m.role));
        }
        if m.content.len() > MAX_CONTENT_BYTES {
            return Err(format!("单条消息超出 {} 字节上限", MAX_CONTENT_BYTES));
        }
    }
    Ok(())
}

/// 流式 chunk 事件载荷（emit 到前端）。
#[derive(Debug, Clone, Serialize)]
pub struct TranslateChunk {
    /// 翻译流携带请求标识；普通 AI 对话流不需要，值为 null。
    pub request_id: Option<u64>,
    /// 本次增量文本。
    pub delta: String,
    /// 是否结束。
    pub done: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn delayed_response(delay: Duration) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(delay).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
        });
        (address, task)
    }

    #[tokio::test]
    async fn clients_with_different_timeouts_do_not_share_the_first_timeout() {
        let (slow_address, slow_task) = delayed_response(Duration::from_millis(80)).await;
        let short = shared_client(Duration::from_millis(10), false);
        assert!(short
            .get(format!("http://{slow_address}"))
            .send()
            .await
            .is_err());
        slow_task.await.unwrap();

        let (fast_address, fast_task) = delayed_response(Duration::from_millis(50)).await;
        let long = shared_client(Duration::from_secs(1), false);
        assert!(long
            .get(format!("http://{fast_address}"))
            .send()
            .await
            .is_ok());
        fast_task.await.unwrap();
    }

    #[test]
    fn rejects_oversized_translation_input() {
        assert!(validate_translation_text(&"x".repeat(MAX_TRANSLATION_TEXT_BYTES + 1)).is_err());
        assert!(validate_translation_text(&"x".repeat(MAX_TRANSLATION_TEXT_BYTES)).is_ok());
    }

    #[test]
    fn limited_body_rejects_chunks_over_the_cap() {
        let mut body = LimitedBody::new(4, "test body");

        assert!(body.push(b"1234").is_ok());
        assert!(body.push(b"5").is_err());
    }

    #[test]
    fn error_body_is_truncated_at_the_cap() {
        let mut body = ErrorBody::new(4);

        assert!(body.push(b"123456"));
        assert_eq!(body.finish(), "1234…[truncated]");
    }

    #[test]
    fn stream_chunks_serialize_the_originating_request_id() {
        let value = serde_json::to_value(TranslateChunk {
            request_id: Some(42),
            delta: "hello".to_string(),
            done: false,
        })
        .unwrap();

        assert_eq!(value["request_id"], 42);
    }

    #[test]
    fn rejects_unknown_or_oversized_language_codes() {
        assert!(validate_language_pair("auto", "zh-CN").is_ok());
        assert!(validate_language_pair("xx-custom", "zh-CN").is_err());
        assert!(validate_language_pair(&"x".repeat(128), "en").is_err());
        assert!(validate_language_pair("en", "auto").is_err());
    }
}
