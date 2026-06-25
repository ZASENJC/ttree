//! OpenAI 兼容流式能力：翻译流 + 普通 AI 对话流。

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};

use crate::config::{ChatAiConfig, OpenAiConfig};
use crate::translate::{shared_client, ChatMessage, TranslateChunk};

/// 流式翻译事件名。
pub const CHUNK_EVENT: &str = "translate-chunk";
/// 流式 AI 对话事件名。
pub const CHAT_CHUNK_EVENT: &str = "chat-chunk";

/// 单次请求的整体超时（含流式），避免网络挂起永久占用 loading 态。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn render_translate_prompt(cfg: &OpenAiConfig, source: &str, target: &str) -> String {
    let src = if source == "auto" {
        "自动检测的源语言".to_string()
    } else {
        source.to_string()
    };
    cfg.translate_prompt
        .replace("{source}", &src)
        .replace("{target}", target)
}

fn build_translate_messages(cfg: &OpenAiConfig, text: &str, source: &str, target: &str) -> Vec<Value> {
    let prompt = render_translate_prompt(cfg, source, target);
    vec![
        json!({ "role": "system", "content": prompt }),
        json!({
            "role": "user",
            "content": format!(
                "请严格执行以下翻译提示词：\n{prompt}\n\n待处理文本：\n{text}"
            )
        }),
    ]
}

/// 调用 OpenAI 兼容接口做流式翻译，每个 delta 通过事件推送前端。
pub async fn translate_stream<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &OpenAiConfig,
    text: &str,
    source: &str,
    target: &str,
) -> Result<(), String> {
    let messages = build_translate_messages(cfg, text, source, target);
    stream_messages(app, &cfg.base_url, &cfg.api_key, &cfg.model, messages, CHUNK_EVENT).await
}

/// 普通 AI 对话：使用独立 AI 对话配置。若配置了自定义提示词则作为 system 消息前置。
pub async fn chat_stream<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &ChatAiConfig,
    messages: &[ChatMessage],
) -> Result<(), String> {
    let mut payload: Vec<Value> = Vec::with_capacity(messages.len() + 1);
    if !cfg.chat_prompt.trim().is_empty() {
        payload.push(json!({ "role": "system", "content": cfg.chat_prompt }));
    }
    payload.extend(
        messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content })),
    );
    stream_messages(
        app,
        &cfg.base_url,
        &cfg.api_key,
        &cfg.model,
        payload,
        CHAT_CHUNK_EVENT,
    )
    .await
}

async fn stream_messages<R: Runtime>(
    app: &AppHandle<R>,
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: Vec<Value>,
    event_name: &str,
) -> Result<(), String> {
    if api_key.is_empty() {
        return Err("未配置 API Key".to_string());
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let payload = json!({
        "model": model,
        "stream": true,
        "messages": messages,
    });

    let resp = shared_client(REQUEST_TIMEOUT, false)
        .post(&url)
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("OpenAI 请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI 返回状态 {status}: {body}"));
    }

    let mut stream = resp.bytes_stream();
    // 以原始字节缓冲，按 '\n' 切出完整行后再解码，避免多字节字符（中文）
    // 跨 chunk 被截断成 U+FFFD。
    let mut buffer: Vec<u8> = Vec::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("读取流失败: {e}"))?;
        buffer.extend_from_slice(&bytes);

        while let Some(pos) = memchr::memchr(b'\n', &buffer) {
            // 先把这一行（含换行符）移出 buffer，再解码，规避借用冲突。
            let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();

            let Ok(line) = std::str::from_utf8(&line_bytes) else {
                continue;
            };
            let line = line.trim();

            if let Some(delta) = parse_sse_line(line) {
                match delta {
                    SseDelta::Content(t) => {
                        let _ = app.emit(
                            event_name,
                            TranslateChunk { delta: t, done: false },
                        );
                    }
                    SseDelta::Done => {
                        let _ = app.emit(
                            event_name,
                            TranslateChunk { delta: String::new(), done: true },
                        );
                        return Ok(());
                    }
                }
            }
        }
    }

    let _ = app.emit(
        event_name,
        TranslateChunk { delta: String::new(), done: true },
    );
    Ok(())
}

/// 单行 SSE 解析结果。
enum SseDelta {
    Content(String),
    Done,
}

/// 解析一行 SSE：`data: {...}` 或 `data: [DONE]`。
fn parse_sse_line(line: &str) -> Option<SseDelta> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(SseDelta::Done);
    }
    let v: Value = serde_json::from_str(data).ok()?;
    let content = v
        .get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()?;
    Some(SseDelta::Content(content.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"你好"}}]}"#;
        match parse_sse_line(line) {
            Some(SseDelta::Content(t)) => assert_eq!(t, "你好"),
            _ => panic!("expected content"),
        }
    }

    #[test]
    fn parses_done() {
        assert!(matches!(parse_sse_line("data: [DONE]"), Some(SseDelta::Done)));
    }

    #[test]
    fn ignores_non_data_lines() {
        assert!(parse_sse_line(": keep-alive").is_none());
        assert!(parse_sse_line("").is_none());
    }

    #[test]
    fn ignores_delta_without_content() {
        let line = r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#;
        assert!(parse_sse_line(line).is_none());
    }

    #[test]
    fn renders_translate_prompt_placeholders() {
        let cfg = OpenAiConfig {
            translate_prompt: "from {source} to {target}".to_string(),
            ..OpenAiConfig::default()
        };
        assert_eq!(
            render_translate_prompt(&cfg, "auto", "zh-CN"),
            "from 自动检测的源语言 to zh-CN"
        );
    }

    #[test]
    fn includes_custom_translate_prompt_in_system_and_user_messages() {
        let cfg = OpenAiConfig {
            translate_prompt: "CUSTOM {source} => {target}".to_string(),
            ..OpenAiConfig::default()
        };
        let messages = build_translate_messages(&cfg, "hello", "en", "zh-CN");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "CUSTOM en => zh-CN");
        assert!(messages[1]["content"].as_str().unwrap().contains("CUSTOM en => zh-CN"));
        assert!(messages[1]["content"].as_str().unwrap().contains("hello"));
    }

    #[test]
    fn prepends_chat_prompt_as_system_message() {
        let prompt = "你是一个友好的助手";
        let history = [ChatMessage {
            role: "user".to_string(),
            content: "你好".to_string(),
        }];

        let mut payload: Vec<Value> = Vec::new();
        if !prompt.trim().is_empty() {
            payload.push(json!({ "role": "system", "content": prompt }));
        }
        payload.extend(
            history
                .iter()
                .map(|m| json!({ "role": m.role, "content": m.content })),
        );

        assert_eq!(payload.len(), 2);
        assert_eq!(payload[0]["role"], "system");
        assert_eq!(payload[0]["content"], prompt);
        assert_eq!(payload[1]["role"], "user");
    }
}
