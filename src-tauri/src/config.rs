//! 配置读写：基于 tauri-plugin-store 持久化用户设置。
//!
//! 存储项：OpenAI base_url / model / 提示词、默认引擎、默认语言对、快捷键。
//! 敏感凭据（api_key）不落盘，改存 macOS Keychain（见 `keychain` 模块）。

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{async_runtime, AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::keychain;

pub const STORE_FILE: &str = "settings.json";
const MAX_BASE_URL_BYTES: usize = 2 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_PROMPT_BYTES: usize = 64 * 1024;
const MAX_API_KEY_BYTES: usize = 16 * 1024;

/// 串行化凭据配置的读取/保存，避免 Store 与 Keychain 两阶段更新期间被并发读取。
static CREDENTIAL_CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn lock_credential_config() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    CREDENTIAL_CONFIG_LOCK
        .lock()
        .map_err(|e| format!("配置锁中毒: {e}"))
}

fn validate_config_field(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.len() > max_bytes {
        return Err(format!("{label}超出 {max_bytes} 字节上限"));
    }
    Ok(())
}

/// 校验 OpenAI 兼容接口的 base_url：
/// - 远程地址必须使用 HTTPS
/// - HTTP 仅允许 localhost / loopback IP，兼容本地 LLM
/// - 拒绝 file:/data:/其它非网络 scheme，避免请求被引向本地文件协议
///
/// 返回规范化后的 URL（去除尾部斜杠），非法则返回 Err。
fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("base_url 不能为空".to_string());
    }
    validate_config_field("base_url", trimmed, MAX_BASE_URL_BYTES)?;
    let url = url::Url::parse(trimmed).map_err(|e| format!("base_url 非法: {e}"))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_url(&url) => {}
        "http" => return Err("远程 base_url 必须使用 HTTPS；HTTP 仅允许本机回环地址".to_string()),
        other => return Err(format!("base_url 必须为 HTTPS，不支持 {other}")),
    }
    if url.host().is_none() {
        return Err("base_url 必须包含主机名或 IP 地址".to_string());
    }
    let mut out = trimmed.trim_end_matches('/').to_string();
    if out.is_empty() {
        out = trimmed.to_string();
    }
    Ok(out)
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

/// OpenAI 兼容接口配置。
#[derive(Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub base_url: String,
    /// 仅作为保存请求中的新密钥；读取配置时始终为空，真实密钥不会发往 WebView。
    #[serde(default, serialize_with = "serialize_empty_api_key")]
    pub api_key: String,
    /// 当前端点是否已有绑定密钥，仅供设置页展示。
    #[serde(default)]
    pub has_api_key: bool,
    /// 保存时显式删除密钥；空 api_key 且 false 表示保留。
    #[serde(default)]
    pub clear_api_key: bool,
    pub model: String,
    pub translate_prompt: String,
}

impl std::fmt::Debug for OpenAiConfig {
    // 脱敏：api_key 永不进入 Debug 输出，避免日志/dbg! 泄漏凭据。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            has_api_key: false,
            clear_api_key: false,
            model: "gpt-4o-mini".to_string(),
            translate_prompt: "你是一个专业翻译引擎。将用户输入从 {source} 翻译成 {target}。只输出译文，不要解释、不要引号、不要附加任何说明。".to_string(),
        }
    }
}

/// AI 对话接口配置（与 AI 翻译分开）。
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatAiConfig {
    pub base_url: String,
    /// 仅作为保存请求中的新密钥；读取配置时始终为空，真实密钥不会发往 WebView。
    #[serde(default, serialize_with = "serialize_empty_api_key")]
    pub api_key: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub clear_api_key: bool,
    pub model: String,
    /// 自定义系统提示词；空字符串表示不附加。
    #[serde(default)]
    pub chat_prompt: String,
}

impl std::fmt::Debug for ChatAiConfig {
    // 脱敏：api_key 永不进入 Debug 输出，避免日志/dbg! 泄漏凭据。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl Default for ChatAiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            has_api_key: false,
            clear_api_key: false,
            model: "gpt-4o-mini".to_string(),
            chat_prompt: String::new(),
        }
    }
}

fn serialize_empty_api_key<S>(_value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str("")
}

/// 全局快捷键配置。空字符串表示该快捷键未启用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutConfig {
    /// 呼出/收起翻译窗口。
    pub toggle: String,
    /// 截图 OCR 翻译。
    pub ocr: String,
    /// 呼出 AI 对话窗口。
    #[serde(default)]
    pub ai_dialog: String,
    /// 划词翻译：捕获选中文本并翻译。
    #[serde(default)]
    pub selection_translate: String,
    /// 划词 AI 对话：捕获选中文本并发送给 AI。
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

/// 外观配置。目前仅含面板透明度（0.0–1.0，默认完全不透明）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// 面板整体透明度，范围 0.0–1.0。低于下限会被钳制。
    #[serde(default = "default_panel_opacity")]
    pub panel_opacity: f64,
}

fn default_panel_opacity() -> f64 {
    1.0
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            panel_opacity: default_panel_opacity(),
        }
    }
}

const PANEL_OPACITY_MIN: f64 = 0.35;
const PANEL_OPACITY_MAX: f64 = 1.0;

impl AppearanceConfig {
    /// 钳制到合法范围。
    pub fn clamp_opacity(value: f64) -> f64 {
        value.clamp(PANEL_OPACITY_MIN, PANEL_OPACITY_MAX)
    }
}

/// 读取用于设置页展示的 OpenAI 配置。真实密钥不会返回给 WebView。
pub fn load_openai<R: Runtime>(app: &AppHandle<R>) -> Result<OpenAiConfig, String> {
    let _guard = lock_credential_config()?;
    load_openai_inner(app, false)
}

/// 读取用于实际请求的 OpenAI 配置，包含与端点绑定的真实密钥。
pub fn load_openai_for_request<R: Runtime>(app: &AppHandle<R>) -> Result<OpenAiConfig, String> {
    let _guard = lock_credential_config()?;
    load_openai_inner(app, true)
}

fn load_openai_inner<R: Runtime>(
    app: &AppHandle<R>,
    include_secret: bool,
) -> Result<OpenAiConfig, String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    let mut cfg = OpenAiConfig::default();
    let persisted_endpoint = normalized_persisted_endpoint(
        store
            .get("openai.base_url")
            .and_then(|v| v.as_str().map(String::from)),
    )?;
    if let Some(endpoint) = persisted_endpoint.as_ref() {
        cfg.base_url = endpoint.clone();
    }
    if let Some(v) = store
        .get("openai.model")
        .and_then(|v| v.as_str().map(String::from))
    {
        if !v.is_empty() {
            cfg.model = v;
        }
    }
    if let Some(v) = store
        .get("openai.translate_prompt")
        .and_then(|v| v.as_str().map(String::from))
    {
        if !v.is_empty() {
            cfg.translate_prompt = v;
        }
    }
    validate_config_field("模型", &cfg.model, MAX_MODEL_BYTES)?;
    validate_config_field("翻译提示词", &cfg.translate_prompt, MAX_PROMPT_BYTES)?;
    let secret = load_secret_migrated(
        app,
        "openai.api_key",
        keychain::ACCOUNT_OPENAI_KEY,
        persisted_endpoint.as_deref(),
    )?;
    cfg.has_api_key = secret.is_some();
    if include_secret {
        cfg.api_key = secret.unwrap_or_default();
    }
    Ok(cfg)
}

/// 保存 OpenAI 配置：非敏感字段写 store，api_key 写 Keychain。
pub fn save_openai<R: Runtime>(app: &AppHandle<R>, cfg: &OpenAiConfig) -> Result<(), String> {
    let _guard = lock_credential_config()?;
    let base_url = normalize_base_url(&cfg.base_url)?;
    validate_config_field("模型", &cfg.model, MAX_MODEL_BYTES)?;
    validate_config_field("翻译提示词", &cfg.translate_prompt, MAX_PROMPT_BYTES)?;
    let update = credential_update(&cfg.api_key, cfg.clear_api_key)?;
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    if update == CredentialUpdate::Preserve {
        let current_endpoint = normalized_persisted_endpoint(
            store
                .get("openai.base_url")
                .and_then(|v| v.as_str().map(String::from)),
        )?;
        let existing = load_secret_migrated(
            app,
            "openai.api_key",
            keychain::ACCOUNT_OPENAI_KEY,
            current_endpoint.as_deref(),
        )?
        .map(ResolvedSecret::Current)
        .unwrap_or(ResolvedSecret::Missing);
        validate_preserved_secret_endpoint(current_endpoint.as_deref(), &base_url, &existing)?;
    }
    let values = [
        (
            "openai.base_url",
            serde_json::Value::String(base_url.clone()),
        ),
        ("openai.model", serde_json::Value::String(cfg.model.clone())),
        (
            "openai.translate_prompt",
            serde_json::Value::String(cfg.translate_prompt.clone()),
        ),
    ];
    save_config_transaction(
        &store,
        keychain::ACCOUNT_OPENAI_KEY,
        &base_url,
        update,
        &values,
        "openai.api_key",
        "保存配置失败",
    )
}

/// 读取用于设置页展示的 AI 对话配置。真实密钥不会返回给 WebView。
pub fn load_chat_ai<R: Runtime>(app: &AppHandle<R>) -> Result<ChatAiConfig, String> {
    let _guard = lock_credential_config()?;
    load_chat_ai_inner(app, false)
}

/// 读取用于实际请求的 AI 对话配置。
pub fn load_chat_ai_for_request<R: Runtime>(app: &AppHandle<R>) -> Result<ChatAiConfig, String> {
    let _guard = lock_credential_config()?;
    load_chat_ai_inner(app, true)
}

fn load_chat_ai_inner<R: Runtime>(
    app: &AppHandle<R>,
    include_secret: bool,
) -> Result<ChatAiConfig, String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    let mut cfg = ChatAiConfig::default();
    let persisted_endpoint = normalized_persisted_endpoint(
        store
            .get("chat_ai.base_url")
            .and_then(|v| v.as_str().map(String::from)),
    )?;
    if let Some(endpoint) = persisted_endpoint.as_ref() {
        cfg.base_url = endpoint.clone();
    }
    if let Some(v) = store
        .get("chat_ai.model")
        .and_then(|v| v.as_str().map(String::from))
    {
        if !v.is_empty() {
            cfg.model = v;
        }
    }
    if let Some(v) = store
        .get("chat_ai.chat_prompt")
        .and_then(|v| v.as_str().map(String::from))
    {
        cfg.chat_prompt = v;
    }
    validate_config_field("模型", &cfg.model, MAX_MODEL_BYTES)?;
    validate_config_field("AI 对话提示词", &cfg.chat_prompt, MAX_PROMPT_BYTES)?;
    let secret = load_secret_migrated(
        app,
        "chat_ai.api_key",
        keychain::ACCOUNT_CHAT_AI_KEY,
        persisted_endpoint.as_deref(),
    )?;
    cfg.has_api_key = secret.is_some();
    if include_secret {
        cfg.api_key = secret.unwrap_or_default();
    }
    Ok(cfg)
}

/// 保存 AI 对话配置：非敏感字段写 store，api_key 写 Keychain。
pub fn save_chat_ai<R: Runtime>(app: &AppHandle<R>, cfg: &ChatAiConfig) -> Result<(), String> {
    let _guard = lock_credential_config()?;
    let base_url = normalize_base_url(&cfg.base_url)?;
    validate_config_field("模型", &cfg.model, MAX_MODEL_BYTES)?;
    validate_config_field("AI 对话提示词", &cfg.chat_prompt, MAX_PROMPT_BYTES)?;
    let update = credential_update(&cfg.api_key, cfg.clear_api_key)?;
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    if update == CredentialUpdate::Preserve {
        let current_endpoint = normalized_persisted_endpoint(
            store
                .get("chat_ai.base_url")
                .and_then(|v| v.as_str().map(String::from)),
        )?;
        let existing = load_secret_migrated(
            app,
            "chat_ai.api_key",
            keychain::ACCOUNT_CHAT_AI_KEY,
            current_endpoint.as_deref(),
        )?
        .map(ResolvedSecret::Current)
        .unwrap_or(ResolvedSecret::Missing);
        validate_preserved_secret_endpoint(current_endpoint.as_deref(), &base_url, &existing)?;
    }
    let values = [
        (
            "chat_ai.base_url",
            serde_json::Value::String(base_url.clone()),
        ),
        (
            "chat_ai.model",
            serde_json::Value::String(cfg.model.clone()),
        ),
        (
            "chat_ai.chat_prompt",
            serde_json::Value::String(cfg.chat_prompt.clone()),
        ),
    ];
    save_config_transaction(
        &store,
        keychain::ACCOUNT_CHAT_AI_KEY,
        &base_url,
        update,
        &values,
        "chat_ai.api_key",
        "保存 AI 对话配置失败",
    )
}

/// 从 store 读取快捷键配置，缺失则回退默认值。
pub fn load_shortcuts<R: Runtime>(app: &AppHandle<R>) -> ShortcutConfig {
    let Ok(store) = app.store(STORE_FILE) else {
        return ShortcutConfig::default();
    };
    let defaults = ShortcutConfig::default();
    ShortcutConfig {
        toggle: persisted_shortcut_value(
            store
                .get("shortcut.toggle")
                .and_then(|v| v.as_str().map(String::from)),
            &defaults.toggle,
        ),
        ocr: persisted_shortcut_value(
            store
                .get("shortcut.ocr")
                .and_then(|v| v.as_str().map(String::from)),
            &defaults.ocr,
        ),
        ai_dialog: persisted_shortcut_value(
            store
                .get("shortcut.ai_dialog")
                .and_then(|v| v.as_str().map(String::from)),
            &defaults.ai_dialog,
        ),
        selection_translate: persisted_shortcut_value(
            store
                .get("shortcut.selection_translate")
                .and_then(|v| v.as_str().map(String::from)),
            &defaults.selection_translate,
        ),
        selection_ai_dialog: persisted_shortcut_value(
            store
                .get("shortcut.selection_ai_dialog")
                .and_then(|v| v.as_str().map(String::from)),
            &defaults.selection_ai_dialog,
        ),
    }
}

fn persisted_shortcut_value(value: Option<String>, default: &str) -> String {
    value.unwrap_or_else(|| default.to_string())
}

/// 保存快捷键配置到 store。
pub fn save_shortcuts<R: Runtime>(app: &AppHandle<R>, cfg: &ShortcutConfig) -> Result<(), String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    store.set("shortcut.toggle", cfg.toggle.as_str());
    store.set("shortcut.ocr", cfg.ocr.as_str());
    store.set("shortcut.ai_dialog", cfg.ai_dialog.as_str());
    store.set(
        "shortcut.selection_translate",
        cfg.selection_translate.as_str(),
    );
    store.set(
        "shortcut.selection_ai_dialog",
        cfg.selection_ai_dialog.as_str(),
    );
    store.save().map_err(|e| format!("保存快捷键失败: {e}"))?;
    Ok(())
}

/// 从 store 读取外观配置，缺失或非法则回退默认值并钳制。
pub fn load_appearance<R: Runtime>(app: &AppHandle<R>) -> AppearanceConfig {
    let mut cfg = AppearanceConfig::default();
    if let Ok(store) = app.store(STORE_FILE) {
        if let Some(v) = store
            .get("appearance.panel_opacity")
            .and_then(|v| v.as_f64())
        {
            cfg.panel_opacity = AppearanceConfig::clamp_opacity(v);
        }
    }
    cfg
}

/// 保存外观配置到 store（写入前先钳制）。
pub fn save_appearance<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &AppearanceConfig,
) -> Result<(), String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    store.set(
        "appearance.panel_opacity",
        AppearanceConfig::clamp_opacity(cfg.panel_opacity),
    );
    store.save().map_err(|e| format!("保存外观配置失败: {e}"))?;
    Ok(())
}

/// 窗口尺寸下限，与 tauri.conf.json 的 minWidth/minHeight 保持一致。
pub const WINDOW_MIN_WIDTH: f64 = 480.0;
pub const WINDOW_MIN_HEIGHT: f64 = 300.0;

/// 读取上次用户调整后的窗口尺寸；缺失或过小则返回 None（沿用配置默认值）。
pub fn load_window_size<R: Runtime>(app: &AppHandle<R>) -> Option<(f64, f64)> {
    let store = app.store(STORE_FILE).ok()?;
    let width = store.get("window.width").and_then(|v| v.as_f64())?;
    let height = store.get("window.height").and_then(|v| v.as_f64())?;
    if width < WINDOW_MIN_WIDTH || height < WINDOW_MIN_HEIGHT {
        return None;
    }
    Some((width, height))
}

#[derive(Default)]
struct WindowSizeQueue {
    pending: Option<(f64, f64)>,
    worker_running: bool,
}

impl WindowSizeQueue {
    /// 返回 true 表示调用方需要启动唯一的落盘 worker。
    fn enqueue(&mut self, size: (f64, f64)) -> bool {
        self.pending = Some(size);
        if self.worker_running {
            return false;
        }
        self.worker_running = true;
        true
    }

    fn take_pending(&mut self) -> Option<(f64, f64)> {
        self.pending.take()
    }

    /// 一轮写盘结束后调用。返回 true 表示期间又有 resize，worker 应继续。
    fn finish_flush(&mut self) -> bool {
        if self.pending.is_some() {
            true
        } else {
            self.worker_running = false;
            false
        }
    }
}

/// 待落盘尺寸与 worker 状态必须受同一把锁保护，避免“取值”和“放行调度”之间丢事件。
static WINDOW_SIZE_QUEUE: Mutex<WindowSizeQueue> = Mutex::new(WindowSizeQueue {
    pending: None,
    worker_running: false,
});
/// 尾部防抖间隔：停止调整 500ms 后才写一次盘。
const SIZE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// 记录用户调整后的窗口尺寸，并防抖落盘。
///
/// `WindowEvent::Resized` 在拖拽过程中每秒触发数十次；若每次都同步 `store.save()`
/// 会在主线程上反复序列化整个 settings.json 并写盘，导致明显卡顿。
/// 这里只更新内存中的最新尺寸，并保证同一时刻最多只有一个落盘任务在排队：
/// 该任务先睡 500ms（吸收后续高频事件），醒来后取出最新值写盘。
/// 写盘在阻塞线程执行，不阻塞主线程。
pub fn save_window_size<R: Runtime>(app: &AppHandle<R>, width: f64, height: f64) {
    let should_start = WINDOW_SIZE_QUEUE
        .lock()
        .map(|mut queue| {
            queue.enqueue((width.max(WINDOW_MIN_WIDTH), height.max(WINDOW_MIN_HEIGHT)))
        })
        .unwrap_or(false);
    if !should_start {
        return;
    }
    let app = app.clone();
    async_runtime::spawn(async move {
        async_runtime::spawn_blocking(move || window_size_worker(&app))
            .await
            .ok();
    });
}

fn window_size_worker<R: Runtime>(app: &AppHandle<R>) {
    loop {
        std::thread::sleep(SIZE_FLUSH_INTERVAL);
        let size = WINDOW_SIZE_QUEUE
            .lock()
            .ok()
            .and_then(|mut queue| queue.take_pending());
        if let Some((width, height)) = size {
            flush_window_size(app, width, height);
        }
        let should_continue = WINDOW_SIZE_QUEUE
            .lock()
            .map(|mut queue| queue.finish_flush())
            .unwrap_or(false);
        if !should_continue {
            break;
        }
    }
}

fn flush_window_size<R: Runtime>(app: &AppHandle<R>, width: f64, height: f64) {
    let Ok(store) = app.store(STORE_FILE) else {
        return;
    };
    store.set("window.width", width);
    store.set("window.height", height);
    if let Err(e) = store.save() {
        eprintln!("[config] 持久化窗口尺寸失败: {e}");
    }
}

// ── Keychain 凭据辅助 ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialUpdate<'a> {
    Preserve,
    Clear,
    Replace(&'a str),
}

fn credential_update(api_key: &str, clear_api_key: bool) -> Result<CredentialUpdate<'_>, String> {
    let api_key = api_key.trim();
    validate_config_field("API Key", api_key, MAX_API_KEY_BYTES)?;
    match (api_key.is_empty(), clear_api_key) {
        (true, false) => Ok(CredentialUpdate::Preserve),
        (true, true) => Ok(CredentialUpdate::Clear),
        (false, false) => Ok(CredentialUpdate::Replace(api_key)),
        (false, true) => Err("不能同时设置新 API Key 和清除 API Key".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedSecret {
    Missing,
    Current(String),
    Legacy(String),
}

fn normalized_persisted_endpoint(raw: Option<String>) -> Result<Option<String>, String> {
    raw.filter(|value| !value.trim().is_empty())
        .map(|value| normalize_base_url(&value))
        .transpose()
}

fn resolve_secret_read(
    endpoint: Option<&str>,
    read: Result<Option<keychain::SecretRecord>, String>,
) -> Result<ResolvedSecret, String> {
    let record = read.map_err(|e| format!("读取 Keychain 失败: {e}"))?;
    match record {
        None => Ok(ResolvedSecret::Missing),
        Some(keychain::SecretRecord::Legacy(value)) => {
            if value.is_empty() {
                return Err("Keychain 中的 API Key 为空".to_string());
            }
            validate_config_field("API Key", &value, MAX_API_KEY_BYTES)?;
            if endpoint.is_none() {
                return Err("配置缺少已持久化接口地址，拒绝使用未绑定的 API Key".to_string());
            }
            Ok(ResolvedSecret::Legacy(value))
        }
        Some(keychain::SecretRecord::Bound {
            endpoint: bound_endpoint,
            value,
        }) => {
            let Some(endpoint) = endpoint else {
                return Err("配置缺少已持久化接口地址，拒绝使用已保存的 API Key".to_string());
            };
            let bound_endpoint = normalize_base_url(&bound_endpoint)
                .map_err(|e| format!("Keychain 绑定的接口地址非法: {e}"))?;
            if bound_endpoint != endpoint {
                return Err("API Key 与当前接口地址不匹配，请重新输入或显式清除密钥".to_string());
            }
            validate_config_field("API Key", &value, MAX_API_KEY_BYTES)?;
            Ok(ResolvedSecret::Current(value))
        }
    }
}

/// 读取 API Key，并把旧版裸 Keychain 值或 store 明文迁移为端点绑定记录。
fn load_secret_migrated<R: Runtime>(
    app: &AppHandle<R>,
    legacy_store_key: &str,
    account: &str,
    endpoint: Option<&str>,
) -> Result<Option<String>, String> {
    match resolve_secret_read(endpoint, keychain::get_secret_record(account))? {
        ResolvedSecret::Current(value) => {
            let store = app
                .store(STORE_FILE)
                .map_err(|e| format!("打开配置失败: {e}"))?;
            clear_legacy_secret(&store, legacy_store_key, "清理旧明文 API Key 失败")?;
            return Ok(Some(value));
        }
        ResolvedSecret::Legacy(value) => {
            let endpoint = endpoint.expect("legacy secret resolution requires an endpoint");
            keychain::set_bound_secret(account, endpoint, &value)
                .map_err(|e| format!("迁移 Keychain({account}) 失败: {e}"))?;
            let store = app
                .store(STORE_FILE)
                .map_err(|e| format!("打开配置失败: {e}"))?;
            clear_legacy_secret(&store, legacy_store_key, "清理旧明文 API Key 失败")?;
            return Ok(Some(value));
        }
        ResolvedSecret::Missing => {}
    }

    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("打开配置失败: {e}"))?;
    let legacy_value = store
        .get(legacy_store_key)
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty());
    let Some(value) = legacy_value else {
        return Ok(None);
    };
    validate_config_field("API Key", &value, MAX_API_KEY_BYTES)?;
    let Some(endpoint) = endpoint else {
        return Err("配置缺少已持久化接口地址，拒绝迁移未绑定的 API Key".to_string());
    };
    keychain::set_bound_secret(account, endpoint, &value)
        .map_err(|e| format!("迁移 Keychain({account}) 失败: {e}"))?;
    clear_legacy_secret(&store, legacy_store_key, "清理旧明文 API Key 失败")?;
    Ok(Some(value))
}

fn validate_preserved_secret_endpoint(
    current_endpoint: Option<&str>,
    new_endpoint: &str,
    secret: &ResolvedSecret,
) -> Result<(), String> {
    if matches!(secret, ResolvedSecret::Missing) {
        return Ok(());
    }
    if current_endpoint == Some(new_endpoint) {
        return Ok(());
    }
    Err("接口地址已改变；请重新输入 API Key，或显式清除已保存的密钥".to_string())
}

fn apply_credential_update(
    account: &str,
    endpoint: &str,
    update: CredentialUpdate<'_>,
) -> Result<(), String> {
    match update {
        CredentialUpdate::Preserve => Ok(()),
        CredentialUpdate::Clear => keychain::delete_secret(account),
        CredentialUpdate::Replace(value) => keychain::set_bound_secret(account, endpoint, value),
    }
}

fn restore_raw_secret(account: &str, value: &Option<String>) -> Result<(), String> {
    match value {
        Some(value) => keychain::set_secret(account, value),
        None => keychain::delete_secret(account),
    }
}

#[derive(Clone)]
struct StoreSnapshot {
    entries: Vec<(String, Option<serde_json::Value>)>,
}

fn snapshot_store<R: Runtime>(
    store: &tauri_plugin_store::Store<R>,
    keys: &[&str],
) -> StoreSnapshot {
    StoreSnapshot {
        entries: keys
            .iter()
            .map(|key| ((*key).to_string(), store.get(key)))
            .collect(),
    }
}

fn restore_store<R: Runtime>(store: &tauri_plugin_store::Store<R>, snapshot: &StoreSnapshot) {
    for (key, value) in &snapshot.entries {
        if let Some(value) = value {
            store.set(key, value.clone());
        } else {
            store.delete(key);
        }
    }
}

fn run_compensating_update<ApplyCredential, CommitStore, RollbackStore, RollbackCredential>(
    mut apply_credential: ApplyCredential,
    mut commit_store: CommitStore,
    mut rollback_store: RollbackStore,
    mut rollback_credential: RollbackCredential,
) -> Result<(), String>
where
    ApplyCredential: FnMut() -> Result<(), String>,
    CommitStore: FnMut() -> Result<(), String>,
    RollbackStore: FnMut() -> Result<(), String>,
    RollbackCredential: FnMut() -> Result<(), String>,
{
    apply_credential()?;
    let Err(commit_error) = commit_store() else {
        return Ok(());
    };

    let store_rollback_error = rollback_store().err();
    let credential_rollback_error = rollback_credential().err();
    let mut message = format!("提交配置失败: {commit_error}");
    if let Some(error) = store_rollback_error {
        message.push_str(&format!("；回滚 Store 失败: {error}"));
    }
    if let Some(error) = credential_rollback_error {
        message.push_str(&format!("；回滚 Keychain 失败: {error}"));
    }
    Err(message)
}

fn save_config_transaction<R: Runtime>(
    store: &tauri_plugin_store::Store<R>,
    account: &str,
    endpoint: &str,
    update: CredentialUpdate<'_>,
    values: &[(&str, serde_json::Value)],
    legacy_store_key: &str,
    error_context: &str,
) -> Result<(), String> {
    let mut keys: Vec<&str> = values.iter().map(|(key, _)| *key).collect();
    keys.push(legacy_store_key);
    let snapshot = snapshot_store(store, &keys);
    let previous_secret =
        keychain::get_secret(account).map_err(|e| format!("读取 Keychain 以准备保存失败: {e}"))?;

    run_compensating_update(
        || {
            apply_credential_update(account, endpoint, update)
                .map_err(|e| format!("更新 Keychain 失败: {e}"))
        },
        || {
            for (key, value) in values {
                store.set(*key, value.clone());
            }
            store.delete(legacy_store_key);
            store.save().map_err(|e| format!("{error_context}: {e}"))
        },
        || {
            restore_store(store, &snapshot);
            store.save().map_err(|e| format!("恢复原配置失败: {e}"))
        },
        || {
            restore_raw_secret(account, &previous_secret)
                .map_err(|e| format!("恢复原 Keychain 凭据失败: {e}"))
        },
    )
}

fn persist_deletion_with_restore<Delete, Persist, Restore>(
    mut delete: Delete,
    mut persist: Persist,
    mut restore: Restore,
) -> Result<(), String>
where
    Delete: FnMut(),
    Persist: FnMut() -> Result<(), String>,
    Restore: FnMut(),
{
    delete();
    if let Err(error) = persist() {
        restore();
        return Err(error);
    }
    Ok(())
}

fn clear_legacy_secret<R: Runtime>(
    store: &tauri_plugin_store::Store<R>,
    legacy_store_key: &str,
    error_context: &str,
) -> Result<(), String> {
    let Some(previous_value) = store.get(legacy_store_key) else {
        return Ok(());
    };
    persist_deletion_with_restore(
        || {
            store.delete(legacy_store_key);
        },
        || store.save().map_err(|e| format!("{error_context}: {e}")),
        || store.set(legacy_store_key, previous_value.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::SecretRecord;
    use std::cell::RefCell;

    #[test]
    fn rejects_plain_http_for_remote_hosts() {
        let error = normalize_base_url("http://api.example.com/v1").unwrap_err();

        assert!(error.contains("HTTPS"));
    }

    #[test]
    fn allows_plain_http_for_loopback_hosts() {
        assert_eq!(
            normalize_base_url("http://localhost:11434/v1/").unwrap(),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8080/v1").unwrap(),
            "http://127.0.0.1:8080/v1"
        );
        assert_eq!(
            normalize_base_url("http://[::1]:8080/v1").unwrap(),
            "http://[::1]:8080/v1"
        );
    }

    #[test]
    fn credential_update_distinguishes_preserve_clear_and_replace() {
        assert_eq!(
            credential_update("", false).unwrap(),
            CredentialUpdate::Preserve
        );
        assert_eq!(
            credential_update("", true).unwrap(),
            CredentialUpdate::Clear
        );
        assert_eq!(
            credential_update("  new-secret  ", false).unwrap(),
            CredentialUpdate::Replace("new-secret")
        );
        assert!(credential_update("new-secret", true).is_err());
    }

    #[test]
    fn serialized_configs_never_expose_real_api_keys() {
        let config = OpenAiConfig {
            api_key: "top-secret".to_string(),
            has_api_key: true,
            ..OpenAiConfig::default()
        };

        let serialized = serde_json::to_value(config).unwrap();
        assert_eq!(serialized["api_key"], "");
        assert!(!serialized.to_string().contains("top-secret"));
    }

    #[test]
    fn keychain_read_errors_fail_closed() {
        let result = resolve_secret_read(
            Some("https://api.example.com/v1"),
            Err("Keychain access denied".to_string()),
        );

        assert!(result.unwrap_err().contains("Keychain access denied"));
    }

    #[test]
    fn bound_secret_must_match_the_persisted_endpoint() {
        let record = SecretRecord::Bound {
            endpoint: "https://old.example.com/v1".to_string(),
            value: "secret".to_string(),
        };

        let error =
            resolve_secret_read(Some("https://new.example.com/v1"), Ok(Some(record))).unwrap_err();
        assert!(error.contains("不匹配"));
    }

    #[test]
    fn secret_without_a_persisted_endpoint_is_rejected() {
        let record = SecretRecord::Legacy("secret".to_string());

        assert!(resolve_secret_read(None, Ok(Some(record))).is_err());
    }

    #[test]
    fn changing_endpoint_requires_replacing_or_clearing_the_key() {
        let existing = ResolvedSecret::Current("secret".to_string());

        assert!(validate_preserved_secret_endpoint(
            Some("https://old.example.com/v1"),
            "https://new.example.com/v1",
            &existing,
        )
        .is_err());
        assert!(validate_preserved_secret_endpoint(
            Some("https://old.example.com/v1"),
            "https://old.example.com/v1",
            &existing,
        )
        .is_ok());
    }

    #[test]
    fn shortcut_empty_values_override_defaults() {
        assert_eq!(
            persisted_shortcut_value(Some(String::new()), "CmdOrCtrl+Shift+Space"),
            ""
        );
        assert_eq!(
            persisted_shortcut_value(None, "CmdOrCtrl+Shift+Space"),
            "CmdOrCtrl+Shift+Space"
        );
    }

    #[test]
    fn window_size_queue_never_loses_resize_during_a_flush() {
        let mut queue = WindowSizeQueue::default();

        assert!(queue.enqueue((640.0, 480.0)));
        assert!(!queue.enqueue((700.0, 500.0)));
        assert_eq!(queue.take_pending(), Some((700.0, 500.0)));

        // A resize that arrives while the worker is writing must keep the same worker alive.
        assert!(!queue.enqueue((800.0, 600.0)));
        assert!(queue.finish_flush());
        assert_eq!(queue.take_pending(), Some((800.0, 600.0)));
        assert!(!queue.finish_flush());

        // Once the worker exits, the next resize must schedule a new worker.
        assert!(queue.enqueue((900.0, 700.0)));
    }

    #[test]
    fn failed_store_commit_runs_both_compensating_rollbacks() {
        let events = RefCell::new(Vec::new());

        let error = run_compensating_update(
            || {
                events.borrow_mut().push("apply-credential");
                Ok(())
            },
            || {
                events.borrow_mut().push("commit-store");
                Err("disk denied".to_string())
            },
            || {
                events.borrow_mut().push("rollback-store");
                Ok(())
            },
            || {
                events.borrow_mut().push("rollback-credential");
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("disk denied"));
        assert_eq!(
            events.into_inner(),
            [
                "apply-credential",
                "commit-store",
                "rollback-store",
                "rollback-credential",
            ]
        );
    }

    #[test]
    fn rejects_oversized_config_fields() {
        assert!(normalize_base_url(&format!(
            "https://example.com/{}",
            "x".repeat(MAX_BASE_URL_BYTES)
        ))
        .is_err());
        assert!(
            validate_config_field("模型", &"x".repeat(MAX_MODEL_BYTES + 1), MAX_MODEL_BYTES)
                .is_err()
        );
        assert!(validate_config_field(
            "提示词",
            &"x".repeat(MAX_PROMPT_BYTES + 1),
            MAX_PROMPT_BYTES,
        )
        .is_err());
        assert!(credential_update(&"x".repeat(MAX_API_KEY_BYTES + 1), false).is_err());
    }

    #[test]
    fn failed_legacy_secret_cleanup_restores_the_cached_value() {
        let events = RefCell::new(Vec::new());

        let error = persist_deletion_with_restore(
            || events.borrow_mut().push("delete"),
            || Err("disk denied".to_string()),
            || events.borrow_mut().push("restore"),
        )
        .unwrap_err();

        assert!(error.contains("disk denied"));
        assert_eq!(events.into_inner(), ["delete", "restore"]);
    }
}
