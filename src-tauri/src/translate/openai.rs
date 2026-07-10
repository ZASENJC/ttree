//! OpenAI 兼容流式能力：翻译流 + 普通 AI 对话流。

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};

use crate::config::{ChatAiConfig, OpenAiConfig};
use crate::translate::{
    read_error_body, shared_client, ChatMessage, TranslateChunk, MAX_SSE_FRAME_BYTES,
    MAX_STREAM_OUTPUT_BYTES,
};

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

fn build_translate_messages(
    cfg: &OpenAiConfig,
    text: &str,
    source: &str,
    target: &str,
) -> Vec<Value> {
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
    request_id: u64,
    text: &str,
    source: &str,
    target: &str,
) -> Result<(), String> {
    let messages = build_translate_messages(cfg, text, source, target);
    stream_messages(
        app,
        &cfg.base_url,
        &cfg.api_key,
        &cfg.model,
        messages,
        CHUNK_EVENT,
        Some(request_id),
    )
    .await
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
        None,
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
    request_id: Option<u64>,
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
        let body = read_error_body(resp).await;
        return Err(format!("OpenAI 返回状态 {status}: {body}"));
    }

    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::with_limits(MAX_SSE_FRAME_BYTES, MAX_STREAM_OUTPUT_BYTES);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("读取流失败: {e}"))?;
        if emit_deltas(app, event_name, request_id, decoder.push(&bytes)?)? {
            return Ok(());
        }
    }

    if emit_deltas(app, event_name, request_id, decoder.finish()?)? {
        Ok(())
    } else {
        Err("OpenAI 流结束但未收到 [DONE]".to_string())
    }
}

/// 单行 SSE 解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
enum SseDelta {
    Content(String),
    Done,
}

/// 解析一行 SSE：`data: {...}` 或 `data: [DONE]`。
fn parse_sse_line(line: &str) -> Result<Option<SseDelta>, String> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(None);
    };
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }
    if data == "[DONE]" {
        return Ok(Some(SseDelta::Done));
    }
    let v: Value =
        serde_json::from_str(data).map_err(|e| format!("OpenAI SSE JSON 解析失败: {e}"))?;
    let content = v
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str);
    Ok(content.map(|content| SseDelta::Content(content.to_string())))
}

struct SseDecoder {
    buffer: Vec<u8>,
    frame_limit: usize,
    output_limit: usize,
    output_bytes: usize,
    saw_done: bool,
}

impl SseDecoder {
    fn with_limits(frame_limit: usize, output_limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            frame_limit,
            output_limit,
            output_bytes: 0,
            saw_done: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseDelta>, String> {
        let mut deltas = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                if let Some(delta) = self.take_line()? {
                    deltas.push(delta);
                }
                continue;
            }
            if self.buffer.len() >= self.frame_limit {
                return Err(format!("OpenAI SSE 单帧超出 {} 字节上限", self.frame_limit));
            }
            self.buffer.push(*byte);
        }
        Ok(deltas)
    }

    fn finish(mut self) -> Result<Vec<SseDelta>, String> {
        let mut deltas = Vec::new();
        if !self.buffer.is_empty() {
            if let Some(delta) = self.take_line()? {
                deltas.push(delta);
            }
        }
        if !self.saw_done {
            return Err("OpenAI 流结束但未收到 [DONE]".to_string());
        }
        Ok(deltas)
    }

    fn take_line(&mut self) -> Result<Option<SseDelta>, String> {
        let line_bytes = std::mem::take(&mut self.buffer);
        let line = std::str::from_utf8(&line_bytes)
            .map_err(|e| format!("OpenAI SSE 包含非法 UTF-8: {e}"))?
            .trim();
        let Some(delta) = parse_sse_line(line)? else {
            return Ok(None);
        };
        match &delta {
            SseDelta::Content(content) => {
                self.output_bytes = self
                    .output_bytes
                    .checked_add(content.len())
                    .ok_or_else(|| "OpenAI 流式输出大小溢出".to_string())?;
                if self.output_bytes > self.output_limit {
                    return Err(format!(
                        "OpenAI 流式输出超出 {} 字节上限",
                        self.output_limit
                    ));
                }
            }
            SseDelta::Done => self.saw_done = true,
        }
        Ok(Some(delta))
    }
}

fn emit_deltas<R: Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    request_id: Option<u64>,
    deltas: Vec<SseDelta>,
) -> Result<bool, String> {
    for delta in deltas {
        match delta {
            SseDelta::Content(delta) => app
                .emit(
                    event_name,
                    TranslateChunk {
                        request_id,
                        delta,
                        done: false,
                    },
                )
                .map_err(|e| format!("发送流式结果失败: {e}"))?,
            SseDelta::Done => {
                app.emit(
                    event_name,
                    TranslateChunk {
                        request_id,
                        delta: String::new(),
                        done: true,
                    },
                )
                .map_err(|e| format!("发送流式完成事件失败: {e}"))?;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"你好"}}]}"#;
        match parse_sse_line(line).unwrap() {
            Some(SseDelta::Content(t)) => assert_eq!(t, "你好"),
            _ => panic!("expected content"),
        }
    }

    #[test]
    fn parses_done() {
        assert!(matches!(
            parse_sse_line("data: [DONE]").unwrap(),
            Some(SseDelta::Done)
        ));
    }

    #[test]
    fn ignores_non_data_lines() {
        assert!(parse_sse_line(": keep-alive").unwrap().is_none());
        assert!(parse_sse_line("").unwrap().is_none());
    }

    #[test]
    fn ignores_delta_without_content() {
        let line = r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#;
        assert!(parse_sse_line(line).unwrap().is_none());
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
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("CUSTOM en => zh-CN"));
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

    #[test]
    fn parses_final_sse_frame_without_a_trailing_newline() {
        let mut decoder = SseDecoder::with_limits(1024, 1024);
        let deltas = decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]")
            .unwrap();
        assert!(matches!(deltas.as_slice(), [SseDelta::Content(text)] if text == "hello"));

        let final_deltas = decoder.finish().unwrap();
        assert!(matches!(final_deltas.as_slice(), [SseDelta::Done]));
    }

    #[test]
    fn rejects_eof_without_done_marker() {
        let mut decoder = SseDecoder::with_limits(1024, 1024);
        decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n")
            .unwrap();

        assert!(decoder.finish().unwrap_err().contains("[DONE]"));
    }

    #[test]
    fn rejects_oversized_sse_frames() {
        let mut decoder = SseDecoder::with_limits(8, 1024);

        assert!(decoder.push(b"data: 123456789").is_err());
    }

    #[test]
    fn rejects_oversized_stream_output() {
        let mut decoder = SseDecoder::with_limits(1024, 3);

        assert!(decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"four\"}}]}\n")
            .is_err());
    }

    #[test]
    fn rejects_malformed_data_frames_instead_of_silently_skipping_them() {
        let mut decoder = SseDecoder::with_limits(1024, 1024);

        assert!(decoder.push(b"data: {broken}\n").is_err());
    }
}
