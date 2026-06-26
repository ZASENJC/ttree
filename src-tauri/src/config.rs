//! 配置读写：基于 tauri-plugin-store 持久化用户设置。
//!
//! 存储项：OpenAI base_url / model / 提示词、默认引擎、默认语言对、快捷键。
//! 敏感凭据（api_key）不落盘，改存 macOS Keychain（见 `keychain` 模块）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{async_runtime, AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::keychain;

pub const STORE_FILE: &str = "settings.json";

/// 校验 OpenAI 兼容接口的 base_url（温和方案）：
/// - 必须是合法的 http(s) URL（允许 http，兼容本地 LLM / 自建网关）
/// - 拒绝 file:/data:/其它非网络 scheme，避免请求被引向本地文件协议
///
/// 返回规范化后的 URL（去除尾部斜杠），非法则返回 Err。
fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("base_url 不能为空".to_string());
    }
    let url = url::Url::parse(trimmed).map_err(|e| format!("base_url 非法: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("base_url 必须为 http/https，不支持 {other}")),
    }
    // 仅校验 scheme，不强制 host（允许 http://localhost、IP、域名）
    let mut out = trimmed.trim_end_matches('/').to_string();
    if out.is_empty() {
        out = trimmed.to_string();
    }
    Ok(out)
}

/// OpenAI 兼容接口配置。
#[derive(Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: String,
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
            model: "gpt-4o-mini".to_string(),
            translate_prompt: "你是一个专业翻译引擎。将用户输入从 {source} 翻译成 {target}。只输出译文，不要解释、不要引号、不要附加任何说明。".to_string(),
        }
    }
}

/// AI 对话接口配置（与 AI 翻译分开）。
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatAiConfig {
    pub base_url: String,
    pub api_key: String,
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
            model: "gpt-4o-mini".to_string(),
            chat_prompt: String::new(),
        }
    }
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

/// 从 store + Keychain 读取 OpenAI 配置，缺失则回退默认值。
pub fn load_openai<R: Runtime>(app: &AppHandle<R>) -> OpenAiConfig {
    let Ok(store) = app.store(STORE_FILE) else {
        return OpenAiConfig {
            api_key: load_secret_migrated(app, "openai.api_key", keychain::ACCOUNT_OPENAI_KEY),
            ..OpenAiConfig::default()
        };
    };
    let mut cfg = OpenAiConfig::default();
    if let Some(v) = store.get("openai.base_url").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.base_url = v;
        }
    }
    if let Some(v) = store.get("openai.model").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.model = v;
        }
    }
    if let Some(v) = store.get("openai.translate_prompt").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.translate_prompt = v;
        }
    }
    // api_key 走 Keychain（含一次性明文迁移）
    cfg.api_key = load_secret_migrated(app, "openai.api_key", keychain::ACCOUNT_OPENAI_KEY);
    cfg
}

/// 保存 OpenAI 配置：非敏感字段写 store，api_key 写 Keychain。
pub fn save_openai<R: Runtime>(app: &AppHandle<R>, cfg: &OpenAiConfig) -> Result<(), String> {
    let base_url = normalize_base_url(&cfg.base_url)?;
    let store = app.store(STORE_FILE).map_err(|e| format!("打开配置失败: {e}"))?;
    save_secret(keychain::ACCOUNT_OPENAI_KEY, cfg.api_key.trim())?;
    store.set("openai.base_url", base_url);
    store.set("openai.model", cfg.model.as_str());
    store.set("openai.translate_prompt", cfg.translate_prompt.as_str());
    // 兼容老版本：仅在 Keychain 写入成功后清掉 store 里残留的明文 api_key。
    store.delete("openai.api_key");
    store.save().map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(())
}

/// 从 store + Keychain 读取 AI 对话配置，缺失则回退默认值。
pub fn load_chat_ai<R: Runtime>(app: &AppHandle<R>) -> ChatAiConfig {
    let Ok(store) = app.store(STORE_FILE) else {
        return ChatAiConfig {
            api_key: load_secret_migrated(app, "chat_ai.api_key", keychain::ACCOUNT_CHAT_AI_KEY),
            ..ChatAiConfig::default()
        };
    };
    let mut cfg = ChatAiConfig::default();
    if let Some(v) = store.get("chat_ai.base_url").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.base_url = v;
        }
    }
    if let Some(v) = store.get("chat_ai.model").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.model = v;
        }
    }
    if let Some(v) = store.get("chat_ai.chat_prompt").and_then(|v| v.as_str().map(String::from)) {
        cfg.chat_prompt = v;
    }
    // api_key 走 Keychain（含一次性明文迁移）
    cfg.api_key = load_secret_migrated(app, "chat_ai.api_key", keychain::ACCOUNT_CHAT_AI_KEY);
    cfg
}

/// 保存 AI 对话配置：非敏感字段写 store，api_key 写 Keychain。
pub fn save_chat_ai<R: Runtime>(app: &AppHandle<R>, cfg: &ChatAiConfig) -> Result<(), String> {
    let base_url = normalize_base_url(&cfg.base_url)?;
    let store = app.store(STORE_FILE).map_err(|e| format!("打开配置失败: {e}"))?;
    save_secret(keychain::ACCOUNT_CHAT_AI_KEY, cfg.api_key.trim())?;
    store.set("chat_ai.base_url", base_url);
    store.set("chat_ai.model", cfg.model.as_str());
    store.set("chat_ai.chat_prompt", cfg.chat_prompt.as_str());
    // 兼容老版本：仅在 Keychain 写入成功后清掉 store 里残留的明文 api_key。
    store.delete("chat_ai.api_key");
    store.save().map_err(|e| format!("保存 AI 对话配置失败: {e}"))?;
    Ok(())
}

/// 从 store 读取快捷键配置，缺失则回退默认值。
pub fn load_shortcuts<R: Runtime>(app: &AppHandle<R>) -> ShortcutConfig {
    let Ok(store) = app.store(STORE_FILE) else {
        return ShortcutConfig::default();
    };
    let mut cfg = ShortcutConfig::default();
    if let Some(v) = store.get("shortcut.toggle").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.toggle = v;
        }
    }
    if let Some(v) = store.get("shortcut.ocr").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.ocr = v;
        }
    }
    if let Some(v) = store
        .get("shortcut.ai_dialog")
        .and_then(|v| v.as_str().map(String::from))
    {
        cfg.ai_dialog = v;
    }
    if let Some(v) = store
        .get("shortcut.selection_translate")
        .and_then(|v| v.as_str().map(String::from))
    {
        cfg.selection_translate = v;
    }
    if let Some(v) = store
        .get("shortcut.selection_ai_dialog")
        .and_then(|v| v.as_str().map(String::from))
    {
        cfg.selection_ai_dialog = v;
    }
    cfg
}

/// 保存快捷键配置到 store。
pub fn save_shortcuts<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &ShortcutConfig,
) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| format!("打开配置失败: {e}"))?;
    store.set("shortcut.toggle", cfg.toggle.as_str());
    store.set("shortcut.ocr", cfg.ocr.as_str());
    store.set("shortcut.ai_dialog", cfg.ai_dialog.as_str());
    store.set("shortcut.selection_translate", cfg.selection_translate.as_str());
    store.set("shortcut.selection_ai_dialog", cfg.selection_ai_dialog.as_str());
    store.save().map_err(|e| format!("保存快捷键失败: {e}"))?;
    Ok(())
}

/// 从 store 读取外观配置，缺失或非法则回退默认值并钳制。
pub fn load_appearance<R: Runtime>(app: &AppHandle<R>) -> AppearanceConfig {
    let mut cfg = AppearanceConfig::default();
    if let Ok(store) = app.store(STORE_FILE) {
        if let Some(v) = store.get("appearance.panel_opacity").and_then(|v| v.as_f64()) {
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
    let store = app.store(STORE_FILE).map_err(|e| format!("打开配置失败: {e}"))?;
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

/// 待落盘的最新窗口尺寸（拖拽过程中高频更新，但只保留最新值）。
static PENDING_SIZE: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// 是否已有一个落盘任务在排队，避免每次 resize 都起新任务导致任务堆积。
static FLUSH_SCHEDULED: AtomicBool = AtomicBool::new(false);
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
    {
        let Ok(mut pending) = PENDING_SIZE.lock() else {
            return;
        };
        *pending = Some((width.max(WINDOW_MIN_WIDTH), height.max(WINDOW_MIN_HEIGHT)));
    }
    // 已有任务在排队 → 它会取走最新值，无需再起任务。
    if FLUSH_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    async_runtime::spawn(async move {
        async_runtime::spawn_blocking(move || flush_window_size_blocking(&app))
            .await
            .ok();
    });
}

/// 取出待落盘尺寸并写盘（仅在尾部防抖延迟后执行一次）。
fn flush_window_size_blocking<R: Runtime>(app: &AppHandle<R>) {
    // 尾部防抖：睡 SIZE_FLUSH_INTERVAL，吸收期间所有 resize 事件，醒来只写最新值。
    std::thread::sleep(SIZE_FLUSH_INTERVAL);
    // 取出待写值并放行调度标志位，让下一次调整可再次排队。
    let size = PENDING_SIZE.lock().ok().and_then(|mut p| p.take());
    FLUSH_SCHEDULED.store(false, Ordering::SeqCst);
    let Some((width, height)) = size else {
        return;
    };
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

/// 读取一个 api_key：优先从 Keychain 取。
///
/// 一次性明文迁移：若 Keychain 里没有，但旧的 store key（`legacy_store_key`）
/// 还残留明文，则搬进 Keychain 并清掉 store 里的明文，保证只迁移一次。
fn load_secret_migrated<R: Runtime>(
    app: &AppHandle<R>,
    legacy_store_key: &str,
    account: &str,
) -> String {
    // 先看 Keychain 是否已有
    match keychain::get_secret(account) {
        Ok(Some(v)) => return v,
        Ok(None) => {}
        Err(e) => {
            eprintln!("[config] 读取 Keychain({account}) 失败: {e}");
            return String::new();
        }
    }
    // Keychain 没有：尝试从旧 store 明文迁移。只开一次 store，避免重复打开。
    let Some(store) = app.store(STORE_FILE).ok() else {
        return String::new();
    };
    let migrated = store
        .get(legacy_store_key)
        .and_then(|v| v.as_str().map(String::from))
        .filter(|v| !v.is_empty());

    let Some(value) = migrated else {
        return String::new();
    };
    if let Err(e) = keychain::set_secret(account, &value) {
        eprintln!("[config] 迁移 Keychain({account}) 失败: {e}");
        // 迁移失败也把明文返回，避免用户 key 丢失；下次仍会重试
        return value;
    }
    // 迁移成功：清掉 store 里的明文（持久化失败需可见，否则明文可能残留磁盘）
    store.delete(legacy_store_key);
    if let Err(e) = store.save() {
        eprintln!("[config] 清理旧明文 api_key({account}) 失败: {e}");
    }
    value
}

/// 保存 api_key 到 Keychain；为空则删除（用户清空了 key）。
fn save_secret(account: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        keychain::delete_secret(account)
    } else {
        keychain::set_secret(account, value)
    }
}
