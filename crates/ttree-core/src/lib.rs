//! `ttree-core` —— 核心领域模型、语言自适应解析、取消令牌、配置模型与对话上下文处理。

pub mod cancel;
pub mod chat;
pub mod config;
pub mod language;

pub use cancel::{CancelSlot, CancelToken};
pub use chat::{recent_chat_context, validate_messages, ChatMessage, Conversation};
pub use config::{
    normalize_base_url, AiBackendKind, AiTranslationConfig, AppearanceConfig, ChatAiConfig,
    ShortcutConfig, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN, WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
pub use language::{
    resolve_adaptive_target, resolve_language_pair, validate_language_pair, LanguageOption,
    ADAPTIVE_TARGET, SOURCE_LANGUAGES, TARGET_LANGUAGES,
};
