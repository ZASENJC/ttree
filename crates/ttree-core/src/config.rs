//! 核心配置数据结构与校验规则。

use serde::{Deserialize, Serialize};

pub const MAX_BASE_URL_BYTES: usize = 2 * 1024;
pub const MAX_MODEL_BYTES: usize = 256;
pub const MAX_PATH_BYTES: usize = 4 * 1024;
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_API_KEY_BYTES: usize = 16 * 1024;

pub const PANEL_OPACITY_MIN: f64 = 0.35;
pub const PANEL_OPACITY_MAX: f64 = 1.0;

pub const TOP_BAR_HEIGHT: f64 = 38.0;

pub const WINDOW_MIN_WIDTH: f64 = 380.0;
pub const WINDOW_MAX_WIDTH: f64 = 880.0;
pub const WINDOW_DEFAULT_WIDTH: f64 = 480.0;

pub const SOURCE_PANE_MIN_HEIGHT: f64 = 68.0;
pub const SOURCE_PANE_MAX_HEIGHT: f64 = 240.0;
pub const SOURCE_PANE_DEFAULT_HEIGHT: f64 = 104.0;

pub const LANG_PANE_MIN_HEIGHT: f64 = 30.0;
pub const LANG_PANE_MAX_HEIGHT: f64 = 56.0;
pub const LANG_PANE_DEFAULT_HEIGHT: f64 = 34.0;

pub const WEB_PANE_MIN_HEIGHT: f64 = 76.0;
pub const WEB_PANE_MAX_HEIGHT: f64 = 400.0;
pub const WEB_PANE_DEFAULT_HEIGHT: f64 = 162.0;

pub const AI_PANE_MIN_HEIGHT: f64 = 76.0;
pub const AI_PANE_MAX_HEIGHT: f64 = 400.0;
pub const AI_PANE_DEFAULT_HEIGHT: f64 = 162.0;

pub const CHAT_INPUT_MIN_HEIGHT: f64 = 68.0;
pub const CHAT_INPUT_MAX_HEIGHT: f64 = 220.0;
pub const CHAT_INPUT_DEFAULT_HEIGHT: f64 = 96.0;

pub const WINDOW_MIN_HEIGHT: f64 =
    TOP_BAR_HEIGHT + SOURCE_PANE_MIN_HEIGHT + LANG_PANE_MIN_HEIGHT + WEB_PANE_MIN_HEIGHT;
pub const WINDOW_MAX_HEIGHT: f64 = TOP_BAR_HEIGHT
    + SOURCE_PANE_MAX_HEIGHT
    + LANG_PANE_MAX_HEIGHT
    + WEB_PANE_MAX_HEIGHT
    + AI_PANE_MAX_HEIGHT;

/// AI 引擎后端类型：支持云端 OpenAI 兼容、本机 Ollama/LM Studio/MLX 服务、以及直接加载 GGUF。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiBackendKind {
    /// 云端 OpenAI 兼容接口（OpenAI / DeepSeek / OpenRouter 等，需 Keychain API Key）
    #[default]
    OpenAiCompatible,
    /// 本机 Ollama 服务（默认 `http://127.0.0.1:11434`，免 API Key，支持 `/api/tags` 枚举模型）
    Ollama,
    /// 本机 LM Studio / MLX Server / llama.cpp 服务（默认 `http://127.0.0.1:1234/v1`，免或可选 Key）
    LocalServer,
    /// 纯 Rust Candle 框架直接加载本地 `.gguf` 量化模型文件（Metal GPU 加速）
    CandleGguf,
}

impl AiBackendKind {
    pub const ALL: [AiBackendKind; 4] = [
        AiBackendKind::OpenAiCompatible,
        AiBackendKind::Ollama,
        AiBackendKind::LocalServer,
        AiBackendKind::CandleGguf,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "云端 API (OpenAI 兼容)",
            Self::Ollama => "本地服务: Ollama",
            Self::LocalServer => "本地服务: LM Studio / MLX",
            Self::CandleGguf => "本地文件: 直接加载 .gguf (Candle Metal)",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "https://api.openai.com/v1",
            Self::Ollama => "http://127.0.0.1:11434",
            Self::LocalServer => "http://127.0.0.1:1234/v1",
            Self::CandleGguf => "",
        }
    }

    pub fn requires_api_key(self) -> bool {
        matches!(self, Self::OpenAiCompatible)
    }
}

fn default_true() -> bool {
    true
}

/// AI 翻译独立配置（支持开关、切换云端/Ollama/LM Studio/MLX/GGUF）。
#[derive(Clone, Serialize, Deserialize)]
pub struct AiTranslationConfig {
    /// 是否在翻译面板右侧启用 AI 翻译对照列。
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub backend: AiBackendKind,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub clear_api_key: bool,
    pub model: String,
    /// 当 `backend == CandleGguf` 时使用的 `.gguf` 权重文件路径。
    #[serde(default)]
    pub gguf_model_path: String,
    /// 当 `backend == CandleGguf` 时使用的 `tokenizer.json` 路径（留空则自动在同目录查找）。
    #[serde(default)]
    pub gguf_tokenizer_path: String,
    pub translate_prompt: String,
}

impl std::fmt::Debug for AiTranslationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiTranslationConfig")
            .field("enabled", &self.enabled)
            .field("backend", &self.backend)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("gguf_model_path", &self.gguf_model_path)
            .finish_non_exhaustive()
    }
}

impl Default for AiTranslationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: AiBackendKind::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            has_api_key: false,
            clear_api_key: false,
            model: "gpt-4o-mini".to_string(),
            gguf_model_path: String::new(),
            gguf_tokenizer_path: String::new(),
            translate_prompt: "你是一个专业、地道的翻译引擎。将用户输入从 {source} 翻译成 {target}。只输出译文，不要解释、不要引号、不要附加任何说明。".to_string(),
        }
    }
}

/// AI 对话配置（与翻译独立）。
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatAiConfig {
    #[serde(default)]
    pub backend: AiBackendKind,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub clear_api_key: bool,
    pub model: String,
    #[serde(default)]
    pub gguf_model_path: String,
    #[serde(default)]
    pub gguf_tokenizer_path: String,
    #[serde(default)]
    pub chat_prompt: String,
}

impl std::fmt::Debug for ChatAiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAiConfig")
            .field("backend", &self.backend)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("gguf_model_path", &self.gguf_model_path)
            .finish_non_exhaustive()
    }
}

impl Default for ChatAiConfig {
    fn default() -> Self {
        Self {
            backend: AiBackendKind::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            has_api_key: false,
            clear_api_key: false,
            model: "gpt-4o-mini".to_string(),
            gguf_model_path: String::new(),
            gguf_tokenizer_path: String::new(),
            chat_prompt: String::new(),
        }
    }
}

/// 全局快捷键配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutConfig {
    pub toggle: String,
    pub ocr: String,
    #[serde(default)]
    pub ai_dialog: String,
    #[serde(default)]
    pub selection_translate: String,
    #[serde(default)]
    pub selection_ai_dialog: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: String::new(),
            selection_ai_dialog: String::new(),
        }
    }
}

/// 外观配置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_panel_opacity")]
    pub panel_opacity: f64,
}

fn default_panel_opacity() -> f64 {
    0.96
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            panel_opacity: default_panel_opacity(),
        }
    }
}

impl AppearanceConfig {
    pub fn clamp_opacity(value: f64) -> f64 {
        if value.is_nan() {
            return default_panel_opacity();
        }
        value.clamp(PANEL_OPACITY_MIN, PANEL_OPACITY_MAX)
    }
}

/// 窗口宽度与各分栏高度记忆配置（每一栏均带有严格的最小/最大高度限制）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaneLayoutConfig {
    pub width: f64,
    pub source_height: f64,
    pub lang_height: f64,
    pub web_height: f64,
    pub ai_height: f64,
    pub chat_input_height: f64,
}

impl Default for PaneLayoutConfig {
    fn default() -> Self {
        Self {
            width: WINDOW_DEFAULT_WIDTH,
            source_height: SOURCE_PANE_DEFAULT_HEIGHT,
            lang_height: LANG_PANE_DEFAULT_HEIGHT,
            web_height: WEB_PANE_DEFAULT_HEIGHT,
            ai_height: AI_PANE_DEFAULT_HEIGHT,
            chat_input_height: CHAT_INPUT_DEFAULT_HEIGHT,
        }
    }
}

fn clamp_dimension(val: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if !val.is_finite() {
        return fallback;
    }
    val.clamp(min, max)
}

impl PaneLayoutConfig {
    pub fn clamped(self) -> Self {
        Self {
            width: clamp_dimension(
                self.width,
                WINDOW_MIN_WIDTH,
                WINDOW_MAX_WIDTH,
                WINDOW_DEFAULT_WIDTH,
            ),
            source_height: clamp_dimension(
                self.source_height,
                SOURCE_PANE_MIN_HEIGHT,
                SOURCE_PANE_MAX_HEIGHT,
                SOURCE_PANE_DEFAULT_HEIGHT,
            ),
            lang_height: clamp_dimension(
                self.lang_height,
                LANG_PANE_MIN_HEIGHT,
                LANG_PANE_MAX_HEIGHT,
                LANG_PANE_DEFAULT_HEIGHT,
            ),
            web_height: clamp_dimension(
                self.web_height,
                WEB_PANE_MIN_HEIGHT,
                WEB_PANE_MAX_HEIGHT,
                WEB_PANE_DEFAULT_HEIGHT,
            ),
            ai_height: clamp_dimension(
                self.ai_height,
                AI_PANE_MIN_HEIGHT,
                AI_PANE_MAX_HEIGHT,
                AI_PANE_DEFAULT_HEIGHT,
            ),
            chat_input_height: clamp_dimension(
                self.chat_input_height,
                CHAT_INPUT_MIN_HEIGHT,
                CHAT_INPUT_MAX_HEIGHT,
                CHAT_INPUT_DEFAULT_HEIGHT,
            ),
        }
    }

    pub fn min_window_height(ai_enabled: bool) -> f64 {
        let base =
            TOP_BAR_HEIGHT + SOURCE_PANE_MIN_HEIGHT + LANG_PANE_MIN_HEIGHT + WEB_PANE_MIN_HEIGHT;
        if ai_enabled {
            base + AI_PANE_MIN_HEIGHT
        } else {
            base
        }
    }

    pub fn max_window_height(ai_enabled: bool) -> f64 {
        let base =
            TOP_BAR_HEIGHT + SOURCE_PANE_MAX_HEIGHT + LANG_PANE_MAX_HEIGHT + WEB_PANE_MAX_HEIGHT;
        if ai_enabled {
            base + AI_PANE_MAX_HEIGHT
        } else {
            base
        }
    }

    /// 计算翻译视图下的总窗口高度（严格等于顶栏 + 各启用分栏高度之和）。
    pub fn translate_window_height(&self, ai_enabled: bool) -> f64 {
        let c = self.clamped();
        let base = TOP_BAR_HEIGHT + c.source_height + c.lang_height + c.web_height;
        if ai_enabled {
            base + c.ai_height
        } else {
            base
        }
    }

    pub fn adjust_source_height(&mut self, delta: f64) {
        self.source_height = clamp_dimension(
            self.source_height + delta,
            SOURCE_PANE_MIN_HEIGHT,
            SOURCE_PANE_MAX_HEIGHT,
            SOURCE_PANE_DEFAULT_HEIGHT,
        );
    }

    pub fn adjust_lang_height(&mut self, delta: f64) {
        self.lang_height = clamp_dimension(
            self.lang_height + delta,
            LANG_PANE_MIN_HEIGHT,
            LANG_PANE_MAX_HEIGHT,
            LANG_PANE_DEFAULT_HEIGHT,
        );
    }

    pub fn adjust_web_height(&mut self, delta: f64) {
        self.web_height = clamp_dimension(
            self.web_height + delta,
            WEB_PANE_MIN_HEIGHT,
            WEB_PANE_MAX_HEIGHT,
            WEB_PANE_DEFAULT_HEIGHT,
        );
    }

    pub fn adjust_ai_height(&mut self, delta: f64) {
        self.ai_height = clamp_dimension(
            self.ai_height + delta,
            AI_PANE_MIN_HEIGHT,
            AI_PANE_MAX_HEIGHT,
            AI_PANE_DEFAULT_HEIGHT,
        );
    }

    pub fn adjust_chat_input_height(&mut self, delta: f64) {
        self.chat_input_height = clamp_dimension(
            self.chat_input_height + delta,
            CHAT_INPUT_MIN_HEIGHT,
            CHAT_INPUT_MAX_HEIGHT,
            CHAT_INPUT_DEFAULT_HEIGHT,
        );
    }

    /// 当用户拖拽窗口边缘调整宽度/高度时，同步更新宽度并按每一栏的最小/最大限制分配高度。
    pub fn apply_window_resize(&mut self, new_width: f64, new_height: f64, ai_enabled: bool) {
        *self = self.clamped();
        self.width = clamp_dimension(
            new_width,
            WINDOW_MIN_WIDTH,
            WINDOW_MAX_WIDTH,
            WINDOW_DEFAULT_WIDTH,
        );
        let min_h = Self::min_window_height(ai_enabled);
        let max_h = Self::max_window_height(ai_enabled);
        let target_h = clamp_dimension(
            new_height,
            min_h,
            max_h,
            self.translate_window_height(ai_enabled),
        );
        let mut rem = target_h - self.translate_window_height(ai_enabled);
        if rem.abs() < 0.5 {
            return;
        }

        if ai_enabled {
            let old = self.ai_height;
            self.adjust_ai_height(rem);
            rem -= self.ai_height - old;
        }
        if rem.abs() >= 0.5 {
            let old = self.web_height;
            self.adjust_web_height(rem);
            rem -= self.web_height - old;
        }
        if rem.abs() >= 0.5 {
            let old = self.source_height;
            self.adjust_source_height(rem);
            rem -= self.source_height - old;
        }
        if rem.abs() >= 0.5 {
            self.adjust_lang_height(rem);
        }
    }
}

pub fn validate_field_len(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.len() > max_bytes {
        return Err(format!("{label}超出 {max_bytes} 字节上限"));
    }
    Ok(())
}

/// 校验并规范化接口地址：
/// - 远程地址要求 HTTPS
/// - 本机回环地址（`localhost` / `127.0.0.1` / `::1`）或局域网本地模型服务允许 HTTP
/// - 拒绝包含用户名/密码（userinfo）或非 HTTP(S) 协议
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("base_url 不能为空".to_string());
    }
    validate_field_len("base_url", trimmed, MAX_BASE_URL_BYTES)?;
    let url = url::Url::parse(trimmed).map_err(|e| format!("base_url 非法: {e}"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("base_url 不允许包含内嵌用户名或密码".to_string());
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_url(&url) => {}
        "http" => return Err("远程 base_url 必须使用 HTTPS；HTTP 仅允许本机回环地址".to_string()),
        other => return Err(format!("base_url 必须为 HTTP(S)，不支持 {other}")),
    }
    if url.host().is_none() {
        return Err("base_url 必须包含主机名或 IP 地址".to_string());
    }
    let out = trimmed.trim_end_matches('/');
    Ok(if out.is_empty() {
        trimmed.to_string()
    } else {
        out.to_string()
    })
}

fn is_loopback_url(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => {
            host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        }
        Some(url::Host::Ipv4(host)) => host.is_loopback(),
        Some(url::Host::Ipv6(host)) => host.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_https_and_loopback_http_urls() {
        assert_eq!(
            normalize_base_url("https://api.openai.com/v1/").unwrap(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_base_url("http://127.0.0.1:11434/").unwrap(),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            normalize_base_url("http://localhost:1234/v1").unwrap(),
            "http://localhost:1234/v1"
        );
    }

    #[test]
    fn rejects_remote_http_and_file_urls() {
        assert!(normalize_base_url("http://example.com/v1").is_err());
        assert!(normalize_base_url("file:///etc/passwd").is_err());
        assert!(normalize_base_url("https://user:pass@example.com/v1").is_err());
    }

    #[test]
    fn clamps_pane_heights_and_window_width_within_bounds() {
        let mut layout = PaneLayoutConfig {
            width: 120.0,
            source_height: 10.0,
            lang_height: 100.0,
            web_height: 999.0,
            ai_height: 20.0,
            chat_input_height: 500.0,
        }
        .clamped();
        assert_eq!(layout.width, WINDOW_MIN_WIDTH);
        assert_eq!(layout.source_height, SOURCE_PANE_MIN_HEIGHT);
        assert_eq!(layout.lang_height, LANG_PANE_MAX_HEIGHT);
        assert_eq!(layout.web_height, WEB_PANE_MAX_HEIGHT);
        assert_eq!(layout.ai_height, AI_PANE_MIN_HEIGHT);
        assert_eq!(layout.chat_input_height, CHAT_INPUT_MAX_HEIGHT);

        layout.adjust_source_height(1000.0);
        assert_eq!(layout.source_height, SOURCE_PANE_MAX_HEIGHT);

        layout.apply_window_resize(520.0, 200.0, true);
        assert_eq!(layout.width, 520.0);
        assert_eq!(
            layout.translate_window_height(true),
            PaneLayoutConfig::min_window_height(true)
        );
    }
}
