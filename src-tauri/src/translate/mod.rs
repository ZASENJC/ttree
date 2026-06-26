//! 翻译引擎：统一分发 + 各引擎实现。

pub mod bing;
pub mod google;
pub mod openai;

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

/// 复用连接池的 HTTP 客户端。
///
/// `cookie_store=true` 时返回一个独立实例（带 cookie jar），否则返回默认实例。
/// 相同参数多次调用复用同一客户端，避免反复握手建连。
pub(crate) fn shared_client(timeout: Duration, cookie_store: bool) -> Client {
    static DEFAULT: OnceLock<Client> = OnceLock::new();
    static COOKIE: OnceLock<Client> = OnceLock::new();

    let cell = if cookie_store { &COOKIE } else { &DEFAULT };
    cell.get_or_init(|| {
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
            return Err(format!(
                "单条消息超出 {} 字节上限",
                MAX_CONTENT_BYTES
            ));
        }
    }
    Ok(())
}

/// 流式 chunk 事件载荷（emit 到前端）。
#[derive(Debug, Clone, Serialize)]
pub struct TranslateChunk {
    /// 本次增量文本。
    pub delta: String,
    /// 是否结束。
    pub done: bool,
}
