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

/// 流式 chunk 事件载荷（emit 到前端）。
#[derive(Debug, Clone, Serialize)]
pub struct TranslateChunk {
    /// 本次增量文本。
    pub delta: String,
    /// 是否结束。
    pub done: bool,
}
