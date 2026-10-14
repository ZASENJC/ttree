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
pub mod hy_mt_manager;
pub mod sse;

use ttree_core::language::language_display_name;
use ttree_core::{
    validate_language_pair, validate_messages, AiBackendKind, AiTranslationConfig, CancelToken,
    ChatAiConfig, ChatMessage, HyMtQuant,
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

fn hy_mt_target_english_name(target: &str) -> &'static str {
    match target {
        "en" => "English",
        "ja" => "Japanese",
        "ko" => "Korean",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "ru" => "Russian",
        "zh-CN" => "Chinese",
        "zh-TW" => "Traditional Chinese",
        _ => "English",
    }
}

/// 组装翻译消息：
/// - 若模型为腾讯混元专用翻译模型 `HY-MT`（内置后端或端口 18082）且使用默认提示词，自动采用 HY-MT 官方单轮 User 指令模板；
/// - 若自定义提示词包含 `{text}` 占位符，渲染为单条 User 消息；
/// - 其余通用大模型：System 放指令，User 仅放待译原文（省去 50% 冗余 Prompt Token）。
pub fn build_translate_messages(
    cfg: &AiTranslationConfig,
    text: &str,
    source: &str,
    target: &str,
) -> Vec<ChatMessage> {
    let trimmed_text = text.trim();
    let tpl = cfg.translate_prompt.trim();

    if tpl.contains("{text}") {
        let rendered = render_translate_prompt(tpl, source, target).replace("{text}", trimmed_text);
        return vec![ChatMessage::user(rendered)];
    }

    let is_hy_mt = cfg.backend == AiBackendKind::BuiltInHyMt
        || cfg.model.to_ascii_lowercase().contains("hy-mt")
        || cfg.gguf_model_path.to_ascii_lowercase().contains("hy-mt")
        || cfg.base_url.contains(":18082");

    let is_default_prompt = tpl.is_empty()
        || tpl == AiTranslationConfig::default().translate_prompt
        || tpl == "你是一个专业翻译引擎。将用户输入从 {source} 翻译成 {target}。只输出译文，不要解释、不要引号、不要附加任何说明。";

    if is_hy_mt && is_default_prompt {
        let user_prompt = match target {
            "zh-CN" => format!(
                "将以下文本翻译为中文，注意只需要输出翻译后的结果，不要额外解释：\n\n{trimmed_text}"
            ),
            "zh-TW" => format!(
                "将以下文本翻译为繁体中文，注意只需要输出翻译后的结果，不要额外解释：\n\n{trimmed_text}"
            ),
            other => {
                let tgt_en = hy_mt_target_english_name(other);
                format!(
                    "Translate the following segment into {tgt_en}, without additional explanation.\n\n{trimmed_text}"
                )
            }
        };
        return vec![ChatMessage::user(user_prompt)];
    }

    let prompt = render_translate_prompt(&cfg.translate_prompt, source, target);
    vec![ChatMessage::system(prompt), ChatMessage::user(trimmed_text)]
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
        cfg.hy_mt_quant,
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
        cfg.hy_mt_quant,
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
    hy_mt_quant: HyMtQuant,
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
        AiBackendKind::BuiltInHyMt => {
            let local_base_url = hy_mt_manager::ensure_server_running(hy_mt_quant, cancel).await?;
            http_provider::stream_openai_compatible(
                &local_base_url,
                None,
                "hy-mt1.5-1.8b",
                messages,
                cancel,
                on_delta,
            )
            .await
        }
        AiBackendKind::OpenAiCompatible => {
            if api_key.trim().is_empty() {
                return Err(
                    "未配置云端 API Key（可在设置中填入，或切换为内置 HY-MT1.5 / 本地模型）"
                        .to_string(),
                );
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
                base_url, opt_key, model, messages, cancel, on_delta,
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

    #[test]
    fn builds_hy_mt_native_single_turn_user_prompt() {
        let cfg = AiTranslationConfig {
            backend: AiBackendKind::LocalServer,
            base_url: "http://127.0.0.1:18082/v1".to_string(),
            model: "/Volumes/STU/Ai/models/HY-MT1.5-1.8B-bf16".to_string(),
            ..AiTranslationConfig::default()
        };
        let to_zh = build_translate_messages(&cfg, "Hello world", "en", "zh-CN");
        assert_eq!(to_zh.len(), 1);
        assert_eq!(to_zh[0].role, "user");
        assert!(to_zh[0].content.starts_with("将以下文本翻译为中文"));
        assert!(to_zh[0].content.ends_with("Hello world"));

        let to_en = build_translate_messages(&cfg, "你好世界", "zh-CN", "en");
        assert_eq!(to_en.len(), 1);
        assert_eq!(to_en[0].role, "user");
        assert!(to_en[0]
            .content
            .starts_with("Translate the following segment into English"));
        assert!(to_en[0].content.ends_with("你好世界"));
    }
}
