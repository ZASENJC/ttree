//! 云端 OpenAI 兼容接口与本地服务（Ollama / LM Studio / MLX Server）HTTP 流式客户端 + 本地模型自动探测。

use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use ttree_core::{AiBackendKind, CancelToken, ChatMessage};

use crate::sse::{LineStreamDecoder, StreamDelta};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const STREAM_TIMEOUT: Duration = Duration::from_secs(90);

fn http_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(STREAM_TIMEOUT)
            .tcp_keepalive(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build AI HTTP client")
    })
}

/// 自动探测本机运行中的 Ollama (`/api/tags`) 或 LM Studio / MLX (`/v1/models`) 模型列表。
pub async fn discover_local_models(
    backend: AiBackendKind,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let base = if base_url.trim().is_empty() {
        backend.default_base_url()
    } else {
        base_url.trim().trim_end_matches('/')
    };

    match backend {
        AiBackendKind::Ollama => {
            let root = base.trim_end_matches("/v1");
            let url = format!("{root}/api/tags");
            let resp = http_client()
                .get(&url)
                .timeout(Duration::from_secs(3))
                .send()
                .await
                .map_err(|e| format!("无法连接本机 Ollama 服务 ({url}): {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Ollama 返回状态 {}", resp.status()));
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| format!("解析 Ollama 模型列表失败: {e}"))?;
            let models = body
                .get("models")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            item.get("name").and_then(Value::as_str).map(String::from)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(models)
        }
        AiBackendKind::LocalServer | AiBackendKind::OpenAiCompatible => {
            let root = if base.ends_with("/v1") {
                base.to_string()
            } else {
                format!("{base}/v1")
            };
            let url = format!("{root}/models");
            let mut req = http_client().get(&url).timeout(Duration::from_secs(3));
            if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
                req = req.bearer_auth(key);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("无法连接本地模型服务 ({url}): {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("模型服务返回状态 {}", resp.status()));
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| format!("解析模型列表失败: {e}"))?;
            let models = body
                .get("data")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.get("id").and_then(Value::as_str).map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(models)
        }
        AiBackendKind::CandleGguf | AiBackendKind::BuiltInHyMt => Ok(Vec::new()),
    }
}

/// 调用 OpenAI 兼容端点（云端或 LM Studio / MLX / llama.cpp）执行 SSE 流式请求。
pub async fn stream_openai_compatible<F>(
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    cancel: &CancelToken,
    on_delta: &mut F,
) -> Result<(), String>
where
    F: FnMut(&str) + Send,
{
    let base = base_url.trim().trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    };

    let is_hy_mt = model.to_ascii_lowercase().contains("hy-mt")
        || base.contains(":18082")
        || base.contains(":28190");
    let payload = if is_hy_mt {
        json!({
            "model": model,
            "stream": true,
            "temperature": 0.7,
            "top_p": 0.6,
            "top_k": 20,
            "repeat_penalty": 1.05,
            "repetition_penalty": 1.05,
            "messages": messages,
        })
    } else {
        json!({
            "model": model,
            "stream": true,
            "temperature": 0.2,
            "messages": messages,
        })
    };

    let mut req = http_client().post(&url).json(&payload);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }

    let resp = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("请求已取消".to_string()),
        r = req.send() => r.map_err(|e| format!("AI 请求失败: {e}"))?,
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("AI 接口返回状态 {status}: {snippet}"));
    }

    let mut stream = resp.bytes_stream();
    let mut decoder = LineStreamDecoder::new_sse();

    loop {
        let next_chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("请求已取消".to_string()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = next_chunk else {
            break;
        };
        let bytes = chunk.map_err(|e| format!("读取 AI 响应流失败: {e}"))?;
        for delta in decoder.push(&bytes)? {
            match delta {
                StreamDelta::Content(text) => on_delta(&text),
                StreamDelta::Done => return Ok(()),
            }
        }
    }

    for delta in decoder.finish()? {
        if let StreamDelta::Content(text) = delta {
            on_delta(&text);
        }
    }
    Ok(())
}

/// 调用本机 Ollama 原生 `/api/chat` NDJSON 流式接口（自动保持模型热加载 `keep_alive: "15m"`）。
pub async fn stream_ollama<F>(
    base_url: &str,
    model: &str,
    messages: &[ChatMessage],
    cancel: &CancelToken,
    on_delta: &mut F,
) -> Result<(), String>
where
    F: FnMut(&str) + Send,
{
    let base = if base_url.trim().is_empty() {
        "http://127.0.0.1:11434"
    } else {
        base_url
            .trim()
            .trim_end_matches('/')
            .trim_end_matches("/v1")
    };
    let url = format!("{base}/api/chat");

    let payload = json!({
        "model": model,
        "stream": true,
        "keep_alive": "15m",
        "options": {
            "temperature": 0.2
        },
        "messages": messages,
    });

    let resp = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("请求已取消".to_string()),
        r = http_client().post(&url).json(&payload).send() => {
            r.map_err(|e| format!("连接本机 Ollama 失败 ({url}): {e}"))?
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("Ollama 返回状态 {status}: {snippet}"));
    }

    let mut stream = resp.bytes_stream();
    let mut decoder = LineStreamDecoder::new_ollama_ndjson();

    loop {
        let next_chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("请求已取消".to_string()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = next_chunk else {
            break;
        };
        let bytes = chunk.map_err(|e| format!("读取 Ollama 流失败: {e}"))?;
        for delta in decoder.push(&bytes)? {
            match delta {
                StreamDelta::Content(text) => on_delta(&text),
                StreamDelta::Done => return Ok(()),
            }
        }
    }

    for delta in decoder.finish()? {
        if let StreamDelta::Content(text) = delta {
            on_delta(&text);
        }
    }
    Ok(())
}
