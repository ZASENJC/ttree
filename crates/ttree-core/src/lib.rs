//! `ttree-core` —— 核心领域模型、语言自适应解析、取消令牌、配置模型与对话上下文处理。

pub mod cancel;
pub mod chat;
pub mod config;
pub mod language;

pub use cancel::{CancelSlot, CancelToken};
pub use chat::{recent_chat_context, validate_messages, ChatMessage, Conversation};
pub use config::{
    normalize_base_url, normalize_ocr_newlines, AiBackendKind, AiTranslationConfig,
    AppearanceConfig, ChatAiConfig, HyMtQuant, PaneLayoutConfig, ShortcutConfig,
    AI_PANE_DEFAULT_HEIGHT, AI_PANE_MAX_HEIGHT, AI_PANE_MIN_HEIGHT, CHAT_INPUT_DEFAULT_HEIGHT,
    CHAT_INPUT_MAX_HEIGHT, CHAT_INPUT_MIN_HEIGHT, LANG_PANE_DEFAULT_HEIGHT, LANG_PANE_MAX_HEIGHT,
    LANG_PANE_MIN_HEIGHT, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN, SOURCE_PANE_DEFAULT_HEIGHT,
    SOURCE_PANE_MAX_HEIGHT, SOURCE_PANE_MIN_HEIGHT, TOP_BAR_HEIGHT, WEB_PANE_DEFAULT_HEIGHT,
    WEB_PANE_MAX_HEIGHT, WEB_PANE_MIN_HEIGHT, WINDOW_DEFAULT_WIDTH, WINDOW_MAX_HEIGHT,
    WINDOW_MAX_WIDTH, WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
pub use language::{
    resolve_adaptive_target, resolve_language_pair, validate_language_pair, LanguageOption,
    ADAPTIVE_TARGET, SOURCE_LANGUAGES, TARGET_LANGUAGES,
};
