//! `ttree-ai` —— 独立重写的 AI 翻译与对话模块。
//!
//! 核心特性：
//! 1. 与 GUI 框架完全解耦，通过回调流式输出增量文本（`on_delta`）并支持 `CancelToken` 毫秒级中断；
//! 2. 消除旧版重复发送两遍 Prompt 的缺陷，大幅降低首字延迟（TTFT）；
//! 3. 支持 4 类模型后端：
//!    - `OpenAiCompatible`：云端 OpenAI / DeepSeek / OpenRouter 兼容接口
//!    - `Ollama`：本机 Ollama 服务（免 Key，支持 `/api/tags` 自动探测本地已装模型）
//!    - `LocalServer`：本机 LM Studio / MLX Server / llama.cpp 服务（免 Key，支持 `/v1/models` 探测）
//!    - `CandleGguf`：基于纯 Rust `candle` (Metal GPU 加速) 直接加载本地 `.gguf` 量化模型，常驻内存复用

pub mod gguf;
pub mod http_provider;
pub mod sse;

use ttree_core::language::language_display_name;
use ttree_core::{
    AiBackendKind, AiTranslationConfig, CancelToken, ChatAiConfig, ChatMessage,
    validate_language_pair, validate_messages,
};

pub use http_provider::discover_local_models;

/// 渲染精简高效的翻译 System Prompt（不再在 User 消息中重复拼接第二遍 Prompt）。
pub fn render_translate_prompt(template: &str, source: &str, target: &str) -> String {
    let src_name = language_display_name(source);
    let tgt_name = language_display_name(target);
    template
        .replace("{source}", src_name)
        .replace("{target}", tgt_name)
}

/// 组装翻译消息：System 放指令，User 仅放待译原文（省去 50% 冗余 Prompt Token）。
pub fn build_translate_messages(
    cfg: &AiTranslationConfig,
    text: &str,
    source: &str,
    target: &str,
) -> Vec<ChatMessage> {
    let prompt = render_translate_prompt(&cfg.translate_prompt, source, target);
    vec![
        ChatMessage::system(prompt),
        ChatMessage::user(text.trim()),
    ]
}

/// 组装 AI 对话消息。
pub fn build_chat_messages(cfg: &ChatAiConfig, history: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut out = Vec::with_capacity(history.len() + 1);
    if !cfg.chat_prompt.trim().is_empty() {
        out.push(ChatMessage::system(cfg.chat_prompt.trim()));
    }
    out.extend_from_slice(history);
    out
}

/// 执行流式 AI 翻译。
pub async fn stream_translate<F>(
    cfg: &AiTranslationConfig,
    text: &str,
    source: &str,
    target: &str,
    cancel: &CancelToken,
    mut on_delta: F,
) -> Result<(), String>
where
    F: FnMut(&str) + Send,
{
    if !cfg.enabled {
        return Ok(());
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    validate_language_pair(source, target)?;
    let messages = build_translate_messages(cfg, trimmed, source, target);

    dispatch_stream(
        cfg.backend,
        &cfg.base_url,
        &cfg.api_key,
        &cfg.model,
        &cfg.gguf_model_path,
        &cfg.gguf_tokenizer_path,
        &messages,
        cancel,
        &mut on_delta,
    )
    .await
}

/// 执行流式 AI 对话。
pub async fn stream_chat<F>(
    cfg: &ChatAiConfig,
    history: &[ChatMessage],
    cancel: &CancelToken,
    mut on_delta: F,
) -> Result<(), String>
where
    F: FnMut(&str) + Send,
{
    if history.is_empty() {
        return Ok(());
    }
    validate_messages(history)?;
    let messages = build_chat_messages(cfg, history);

    dispatch_stream(
        cfg.backend,
        &cfg.base_url,
        &cfg.api_key,
        &cfg.model,
        &cfg.gguf_model_path,
        &cfg.gguf_tokenizer_path,
        &messages,
        cancel,
        &mut on_delta,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_stream<F>(
    backend: AiBackendKind,
    base_url: &str,
    api_key: &str,
    model: &str,
    gguf_model_path: &str,
    gguf_tokenizer_path: &str,
    messages: &[ChatMessage],
    cancel: &CancelToken,
    on_delta: &mut F,
) -> Result<(), String>
where
    F: FnMut(&str) + Send,
{
    match backend {
        AiBackendKind::OpenAiCompatible => {
            if api_key.trim().is_empty() {
                return Err("未配置云端 API Key（可在设置中填入，或切换为本地 Ollama / LM Studio / GGUF 模型）".to_string());
            }
            http_provider::stream_openai_compatible(
                base_url,
                Some(api_key.trim()),
                model,
                messages,
                cancel,
                on_delta,
            )
            .await
        }
        AiBackendKind::Ollama => {
            http_provider::stream_ollama(base_url, model, messages, cancel, on_delta).await
        }
        AiBackendKind::LocalServer => {
            let opt_key = (!api_key.trim().is_empty()).then_some(api_key.trim());
            http_provider::stream_openai_compatible(
                base_url,
                opt_key,
                model,
                messages,
                cancel,
                on_delta,
            )
            .await
        }
        AiBackendKind::CandleGguf => {
            gguf::stream_gguf_inference(
                gguf_model_path,
                gguf_tokenizer_path,
                messages,
                cancel,
                on_delta,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_compact_translate_messages_without_duplicate_prompt() {
        let cfg = AiTranslationConfig::default();
        let msgs = build_translate_messages(&cfg, "Hello world", "en", "zh-CN");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("简体中文"));
        assert_eq!(msgs[1].role, "user");
        // User 消息仅包含待译原文，不再重复拼接 system prompt
        assert_eq!(msgs[1].content, "Hello world");
    }
}
