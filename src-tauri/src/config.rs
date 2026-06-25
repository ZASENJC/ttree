//! 配置读写：基于 tauri-plugin-store 持久化用户设置。
//!
//! 存储项：OpenAI base_url / api_key / model、默认引擎、默认语言对、快捷键。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

pub const STORE_FILE: &str = "settings.json";

/// OpenAI 兼容接口配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub translate_prompt: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 自定义系统提示词；空字符串表示不附加。
    #[serde(default)]
    pub chat_prompt: String,
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

/// 从 store 读取 OpenAI 配置，缺失则回退默认值。
pub fn load_openai<R: Runtime>(app: &AppHandle<R>) -> OpenAiConfig {
    let Ok(store) = app.store(STORE_FILE) else {
        return OpenAiConfig::default();
    };
    let mut cfg = OpenAiConfig::default();
    if let Some(v) = store.get("openai.base_url").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.base_url = v;
        }
    }
    if let Some(v) = store.get("openai.api_key").and_then(|v| v.as_str().map(String::from)) {
        cfg.api_key = v;
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
    cfg
}

/// 保存 OpenAI 配置到 store。
pub fn save_openai<R: Runtime>(app: &AppHandle<R>, cfg: &OpenAiConfig) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| format!("打开配置失败: {e}"))?;
    store.set("openai.base_url", cfg.base_url.clone());
    store.set("openai.api_key", cfg.api_key.clone());
    store.set("openai.model", cfg.model.clone());
    store.set("openai.translate_prompt", cfg.translate_prompt.clone());
    store.save().map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(())
}

/// 从 store 读取 AI 对话配置，缺失则回退默认值。
pub fn load_chat_ai<R: Runtime>(app: &AppHandle<R>) -> ChatAiConfig {
    let Ok(store) = app.store(STORE_FILE) else {
        return ChatAiConfig::default();
    };
    let mut cfg = ChatAiConfig::default();
    if let Some(v) = store.get("chat_ai.base_url").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.base_url = v;
        }
    }
    if let Some(v) = store.get("chat_ai.api_key").and_then(|v| v.as_str().map(String::from)) {
        cfg.api_key = v;
    }
    if let Some(v) = store.get("chat_ai.model").and_then(|v| v.as_str().map(String::from)) {
        if !v.is_empty() {
            cfg.model = v;
        }
    }
    if let Some(v) = store.get("chat_ai.chat_prompt").and_then(|v| v.as_str().map(String::from)) {
        cfg.chat_prompt = v;
    }
    cfg
}

/// 保存 AI 对话配置到 store。
pub fn save_chat_ai<R: Runtime>(app: &AppHandle<R>, cfg: &ChatAiConfig) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| format!("打开配置失败: {e}"))?;
    store.set("chat_ai.base_url", cfg.base_url.clone());
    store.set("chat_ai.api_key", cfg.api_key.clone());
    store.set("chat_ai.model", cfg.model.clone());
    store.set("chat_ai.chat_prompt", cfg.chat_prompt.clone());
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
    store.set("shortcut.toggle", cfg.toggle.clone());
    store.set("shortcut.ocr", cfg.ocr.clone());
    store.set("shortcut.ai_dialog", cfg.ai_dialog.clone());
    store.set("shortcut.selection_translate", cfg.selection_translate.clone());
    store.set("shortcut.selection_ai_dialog", cfg.selection_ai_dialog.clone());
    store.save().map_err(|e| format!("保存快捷键失败: {e}"))?;
    Ok(())
}
