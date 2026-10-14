//! 内存缓存 + 磁盘持久化的配置管理。
//!
//! 关键性能优化：
//! - 启动时一次性加载 `settings.json` 与 Keychain 密钥到内存 `RwLock`
//! - 翻译/对话/快捷键分发热路径直接从内存读取（0 磁盘 I/O，0 Keychain IPC）
//! - 100% 兼容旧版 Tauri `tauri-plugin-store` 生成的扁平点号键名（如 `"openai.base_url"`）

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::{Map, Value};
use ttree_core::config::{
    validate_field_len, MAX_API_KEY_BYTES, MAX_MODEL_BYTES, MAX_PATH_BYTES, MAX_PROMPT_BYTES,
};
use ttree_core::{
    normalize_base_url, AiBackendKind, AiTranslationConfig, AppearanceConfig, ChatAiConfig,
    ShortcutConfig, WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
use ttree_platform_macos::keychain::{self, ACCOUNT_CHAT_AI_KEY, ACCOUNT_OPENAI_KEY};

pub const APP_BUNDLE_ID: &str = "com.samwstu.ttree";
pub const STORE_FILE: &str = "settings.json";

#[derive(Clone)]
struct CachedSettings {
    raw_map: Map<String, Value>,
    ai_translate: AiTranslationConfig,
    chat_ai: ChatAiConfig,
    shortcuts: ShortcutConfig,
    appearance: AppearanceConfig,
    window_size: (f64, f64),
}

pub struct SettingsStore {
    path: PathBuf,
    cache: RwLock<CachedSettings>,
}

impl SettingsStore {
    pub fn default_config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(APP_BUNDLE_ID)
    }

    pub fn open_default() -> Self {
        let dir = Self::default_config_dir();
        let _ = fs::create_dir_all(&dir);
        Self::open_at(dir.join(STORE_FILE))
    }

    pub fn open_at(path: PathBuf) -> Self {
        let raw_map = read_json_map(&path);
        let ai_translate = load_ai_translation(&raw_map);
        let chat_ai = load_chat_ai(&raw_map);
        let shortcuts = load_shortcuts(&raw_map);
        let appearance = load_appearance(&raw_map);
        let window_size = load_window_size(&raw_map).unwrap_or((680.0, 520.0));

        Self {
            path,
            cache: RwLock::new(CachedSettings {
                raw_map,
                ai_translate,
                chat_ai,
                shortcuts,
                appearance,
                window_size,
            }),
        }
    }

    /// 供翻译请求直接读取（含内存缓存的真实 API Key，零 I/O）。
    pub fn ai_translation_for_request(&self) -> AiTranslationConfig {
        self.cache.read().expect("settings lock poisoned").ai_translate.clone()
    }

    /// 供对话请求直接读取（含内存缓存的真实 API Key，零 I/O）。
    pub fn chat_ai_for_request(&self) -> ChatAiConfig {
        self.cache.read().expect("settings lock poisoned").chat_ai.clone()
    }

    pub fn shortcuts(&self) -> ShortcutConfig {
        self.cache.read().expect("settings lock poisoned").shortcuts.clone()
    }

    pub fn appearance(&self) -> AppearanceConfig {
        self.cache.read().expect("settings lock poisoned").appearance
    }

    pub fn window_size(&self) -> (f64, f64) {
        self.cache.read().expect("settings lock poisoned").window_size
    }

    pub fn save_ai_translation(&self, input: &AiTranslationConfig) -> Result<(), String> {
        validate_field_len("模型", &input.model, MAX_MODEL_BYTES)?;
        validate_field_len("GGUF 模型路径", &input.gguf_model_path, MAX_PATH_BYTES)?;
        validate_field_len("Tokenizer 路径", &input.gguf_tokenizer_path, MAX_PATH_BYTES)?;
        validate_field_len("翻译提示词", &input.translate_prompt, MAX_PROMPT_BYTES)?;

        let base_url = if input.backend == AiBackendKind::CandleGguf && input.base_url.trim().is_empty() {
            String::new()
        } else {
            normalize_base_url(&input.base_url)?
        };

        let mut guard = self.cache.write().expect("settings lock poisoned");
        let resolved_key = update_secret_if_needed(
            ACCOUNT_OPENAI_KEY,
            &base_url,
            &guard.ai_translate.api_key,
            &input.api_key,
            input.clear_api_key,
        )?;

        let updated = AiTranslationConfig {
            enabled: input.enabled,
            backend: input.backend,
            base_url: base_url.clone(),
            has_api_key: !resolved_key.is_empty(),
            api_key: resolved_key,
            clear_api_key: false,
            model: input.model.trim().to_string(),
            gguf_model_path: input.gguf_model_path.trim().to_string(),
            gguf_tokenizer_path: input.gguf_tokenizer_path.trim().to_string(),
            translate_prompt: input.translate_prompt.clone(),
        };

        guard.raw_map.insert("openai.enabled".into(), Value::Bool(updated.enabled));
        guard.raw_map.insert(
            "openai.backend".into(),
            serde_json::to_value(updated.backend).unwrap_or(Value::String("open_ai_compatible".into())),
        );
        guard.raw_map.insert("openai.base_url".into(), Value::String(updated.base_url.clone()));
        guard.raw_map.insert("openai.model".into(), Value::String(updated.model.clone()));
        guard.raw_map.insert(
            "openai.gguf_model_path".into(),
            Value::String(updated.gguf_model_path.clone()),
        );
        guard.raw_map.insert(
            "openai.gguf_tokenizer_path".into(),
            Value::String(updated.gguf_tokenizer_path.clone()),
        );
        guard.raw_map.insert(
            "openai.translate_prompt".into(),
            Value::String(updated.translate_prompt.clone()),
        );
        guard.raw_map.remove("openai.api_key");

        write_json_map(&self.path, &guard.raw_map)?;
        guard.ai_translate = updated;
        Ok(())
    }

    pub fn save_chat_ai(&self, input: &ChatAiConfig) -> Result<(), String> {
        validate_field_len("模型", &input.model, MAX_MODEL_BYTES)?;
        validate_field_len("GGUF 模型路径", &input.gguf_model_path, MAX_PATH_BYTES)?;
        validate_field_len("Tokenizer 路径", &input.gguf_tokenizer_path, MAX_PATH_BYTES)?;
        validate_field_len("AI 对话提示词", &input.chat_prompt, MAX_PROMPT_BYTES)?;

        let base_url = if input.backend == AiBackendKind::CandleGguf && input.base_url.trim().is_empty() {
            String::new()
        } else {
            normalize_base_url(&input.base_url)?
        };

        let mut guard = self.cache.write().expect("settings lock poisoned");
        let resolved_key = update_secret_if_needed(
            ACCOUNT_CHAT_AI_KEY,
            &base_url,
            &guard.chat_ai.api_key,
            &input.api_key,
            input.clear_api_key,
        )?;

        let updated = ChatAiConfig {
            backend: input.backend,
            base_url: base_url.clone(),
            has_api_key: !resolved_key.is_empty(),
            api_key: resolved_key,
            clear_api_key: false,
            model: input.model.trim().to_string(),
            gguf_model_path: input.gguf_model_path.trim().to_string(),
            gguf_tokenizer_path: input.gguf_tokenizer_path.trim().to_string(),
            chat_prompt: input.chat_prompt.clone(),
        };

        guard.raw_map.insert(
            "chat_ai.backend".into(),
            serde_json::to_value(updated.backend).unwrap_or(Value::String("open_ai_compatible".into())),
        );
        guard.raw_map.insert("chat_ai.base_url".into(), Value::String(updated.base_url.clone()));
        guard.raw_map.insert("chat_ai.model".into(), Value::String(updated.model.clone()));
        guard.raw_map.insert(
            "chat_ai.gguf_model_path".into(),
            Value::String(updated.gguf_model_path.clone()),
        );
        guard.raw_map.insert(
            "chat_ai.gguf_tokenizer_path".into(),
            Value::String(updated.gguf_tokenizer_path.clone()),
        );
        guard.raw_map.insert(
            "chat_ai.chat_prompt".into(),
            Value::String(updated.chat_prompt.clone()),
        );
        guard.raw_map.remove("chat_ai.api_key");

        write_json_map(&self.path, &guard.raw_map)?;
        guard.chat_ai = updated;
        Ok(())
    }

    pub fn save_shortcuts(&self, cfg: &ShortcutConfig) -> Result<(), String> {
        let mut guard = self.cache.write().expect("settings lock poisoned");
        guard.raw_map.insert("shortcut.toggle".into(), Value::String(cfg.toggle.clone()));
        guard.raw_map.insert("shortcut.ocr".into(), Value::String(cfg.ocr.clone()));
        guard.raw_map.insert("shortcut.ai_dialog".into(), Value::String(cfg.ai_dialog.clone()));
        guard.raw_map.insert(
            "shortcut.selection_translate".into(),
            Value::String(cfg.selection_translate.clone()),
        );
        guard.raw_map.insert(
            "shortcut.selection_ai_dialog".into(),
            Value::String(cfg.selection_ai_dialog.clone()),
        );
        write_json_map(&self.path, &guard.raw_map)?;
        guard.shortcuts = cfg.clone();
        Ok(())
    }

    pub fn save_appearance(&self, cfg: AppearanceConfig) -> Result<(), String> {
        let clamped = AppearanceConfig {
            panel_opacity: AppearanceConfig::clamp_opacity(cfg.panel_opacity),
        };
        let mut guard = self.cache.write().expect("settings lock poisoned");
        guard
            .raw_map
            .insert("appearance.panel_opacity".into(), Value::from(clamped.panel_opacity));
        write_json_map(&self.path, &guard.raw_map)?;
        guard.appearance = clamped;
        Ok(())
    }

    pub fn save_window_size(&self, width: f64, height: f64) {
        let w = width.max(WINDOW_MIN_WIDTH);
        let h = height.max(WINDOW_MIN_HEIGHT);
        if let Ok(mut guard) = self.cache.write() {
            if (guard.window_size.0 - w).abs() < 1.0 && (guard.window_size.1 - h).abs() < 1.0 {
                return;
            }
            guard.window_size = (w, h);
            guard.raw_map.insert("window.width".into(), Value::from(w));
            guard.raw_map.insert("window.height".into(), Value::from(h));
            let _ = write_json_map(&self.path, &guard.raw_map);
        }
    }
}

fn update_secret_if_needed(
    account: &str,
    endpoint: &str,
    existing: &str,
    new_key: &str,
    clear: bool,
) -> Result<String, String> {
    let trimmed = new_key.trim();
    validate_field_len("API Key", trimmed, MAX_API_KEY_BYTES)?;
    if clear {
        let _ = keychain::delete_secret(account);
        return Ok(String::new());
    }
    if !trimmed.is_empty() {
        let bind_url = if endpoint.is_empty() {
            "local://gguf"
        } else {
            endpoint
        };
        let _ = keychain::set_bound_secret(account, bind_url, trimmed);
        return Ok(trimmed.to_string());
    }
    Ok(existing.to_string())
}

fn read_json_map(path: &Path) -> Map<String, Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok())
        .unwrap_or_default()
}

fn write_json_map(path: &Path, map: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(map).map_err(|e| format!("序列化配置失败: {e}"))?;
    fs::write(path, bytes).map_err(|e| format!("写入配置文件失败: {e}"))
}

fn load_ai_translation(map: &Map<String, Value>) -> AiTranslationConfig {
    let mut cfg = AiTranslationConfig::default();
    if let Some(enabled) = map.get("openai.enabled").and_then(Value::as_bool) {
        cfg.enabled = enabled;
    }
    if let Some(backend) = map
        .get("openai.backend")
        .and_then(|v| serde_json::from_value::<AiBackendKind>(v.clone()).ok())
    {
        cfg.backend = backend;
    }
    if let Some(url) = map.get("openai.base_url").and_then(Value::as_str) {
        if let Ok(norm) = normalize_base_url(url) {
            cfg.base_url = norm;
        }
    }
    if let Some(m) = map.get("openai.model").and_then(Value::as_str) {
        if !m.trim().is_empty() {
            cfg.model = m.trim().to_string();
        }
    }
    if let Some(p) = map.get("openai.gguf_model_path").and_then(Value::as_str) {
        cfg.gguf_model_path = p.trim().to_string();
    }
    if let Some(p) = map.get("openai.gguf_tokenizer_path").and_then(Value::as_str) {
        cfg.gguf_tokenizer_path = p.trim().to_string();
    }
    if let Some(prompt) = map.get("openai.translate_prompt").and_then(Value::as_str) {
        if !prompt.trim().is_empty() {
            cfg.translate_prompt = prompt.to_string();
        }
    }
    if let Ok(Some(record)) = keychain::get_secret_record(ACCOUNT_OPENAI_KEY) {
        cfg.api_key = record.value().to_string();
        cfg.has_api_key = !cfg.api_key.is_empty();
    }
    cfg
}

fn load_chat_ai(map: &Map<String, Value>) -> ChatAiConfig {
    let mut cfg = ChatAiConfig::default();
    if let Some(backend) = map
        .get("chat_ai.backend")
        .and_then(|v| serde_json::from_value::<AiBackendKind>(v.clone()).ok())
    {
        cfg.backend = backend;
    }
    if let Some(url) = map.get("chat_ai.base_url").and_then(Value::as_str) {
        if let Ok(norm) = normalize_base_url(url) {
            cfg.base_url = norm;
        }
    }
    if let Some(m) = map.get("chat_ai.model").and_then(Value::as_str) {
        if !m.trim().is_empty() {
            cfg.model = m.trim().to_string();
        }
    }
    if let Some(p) = map.get("chat_ai.gguf_model_path").and_then(Value::as_str) {
        cfg.gguf_model_path = p.trim().to_string();
    }
    if let Some(p) = map.get("chat_ai.gguf_tokenizer_path").and_then(Value::as_str) {
        cfg.gguf_tokenizer_path = p.trim().to_string();
    }
    if let Some(prompt) = map.get("chat_ai.chat_prompt").and_then(Value::as_str) {
        cfg.chat_prompt = prompt.to_string();
    }
    if let Ok(Some(record)) = keychain::get_secret_record(ACCOUNT_CHAT_AI_KEY) {
        cfg.api_key = record.value().to_string();
        cfg.has_api_key = !cfg.api_key.is_empty();
    }
    cfg
}

fn load_shortcuts(map: &Map<String, Value>) -> ShortcutConfig {
    let defaults = ShortcutConfig::default();
    ShortcutConfig {
        toggle: map
            .get("shortcut.toggle")
            .and_then(Value::as_str)
            .unwrap_or(&defaults.toggle)
            .to_string(),
        ocr: map
            .get("shortcut.ocr")
            .and_then(Value::as_str)
            .unwrap_or(&defaults.ocr)
            .to_string(),
        ai_dialog: map
            .get("shortcut.ai_dialog")
            .and_then(Value::as_str)
            .unwrap_or(&defaults.ai_dialog)
            .to_string(),
        selection_translate: map
            .get("shortcut.selection_translate")
            .and_then(Value::as_str)
            .unwrap_or(&defaults.selection_translate)
            .to_string(),
        selection_ai_dialog: map
            .get("shortcut.selection_ai_dialog")
            .and_then(Value::as_str)
            .unwrap_or(&defaults.selection_ai_dialog)
            .to_string(),
    }
}

fn load_appearance(map: &Map<String, Value>) -> AppearanceConfig {
    let mut cfg = AppearanceConfig::default();
    if let Some(v) = map.get("appearance.panel_opacity").and_then(Value::as_f64) {
        cfg.panel_opacity = AppearanceConfig::clamp_opacity(v);
    }
    cfg
}

fn load_window_size(map: &Map<String, Value>) -> Option<(f64, f64)> {
    let w = map.get("window.width").and_then(Value::as_f64)?;
    let h = map.get("window.height").and_then(Value::as_f64)?;
    if w < WINDOW_MIN_WIDTH || h < WINDOW_MIN_HEIGHT {
        return None;
    }
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_reloads_settings_via_memory_cache() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = SettingsStore::open_at(tmp.path().to_path_buf());

        let mut cfg = store.ai_translation_for_request();
        cfg.backend = AiBackendKind::Ollama;
        cfg.base_url = "http://127.0.0.1:11434".to_string();
        cfg.model = "qwen2.5:3b".to_string();
        store.save_ai_translation(&cfg).unwrap();

        let reloaded = SettingsStore::open_at(tmp.path().to_path_buf());
        let loaded_cfg = reloaded.ai_translation_for_request();
        assert_eq!(loaded_cfg.backend, AiBackendKind::Ollama);
        assert_eq!(loaded_cfg.base_url, "http://127.0.0.1:11434");
        assert_eq!(loaded_cfg.model, "qwen2.5:3b");
    }
}
