//! 暴露给前端的 Tauri command。

use tauri::{AppHandle, Runtime};

use crate::config::{self, ChatAiConfig, OpenAiConfig, ShortcutConfig};
use crate::ocr;
use crate::screenshot;
use crate::shortcut;
use crate::translate::{self, ChatMessage, Engine, TranslateRequest};
use crate::window;

/// 读取 OpenAI 配置。
#[tauri::command]
pub fn get_openai_config<R: Runtime>(app: AppHandle<R>) -> OpenAiConfig {
    config::load_openai(&app)
}

/// 保存 OpenAI 配置。
#[tauri::command]
pub fn set_openai_config<R: Runtime>(
    app: AppHandle<R>,
    config: OpenAiConfig,
) -> Result<(), String> {
    crate::config::save_openai(&app, &config)
}

/// 读取 AI 对话配置。
#[tauri::command]
pub fn get_chat_ai_config<R: Runtime>(app: AppHandle<R>) -> ChatAiConfig {
    config::load_chat_ai(&app)
}

/// 保存 AI 对话配置。
#[tauri::command]
pub fn set_chat_ai_config<R: Runtime>(
    app: AppHandle<R>,
    config: ChatAiConfig,
) -> Result<(), String> {
    crate::config::save_chat_ai(&app, &config)
}

/// 读取快捷键配置。
#[tauri::command]
pub fn get_shortcut_config<R: Runtime>(app: AppHandle<R>) -> ShortcutConfig {
    config::load_shortcuts(&app)
}

/// 保存快捷键配置并立即重注册。
#[tauri::command]
pub fn set_shortcut_config<R: Runtime>(
    app: AppHandle<R>,
    config: ShortcutConfig,
) -> Result<(), String> {
    shortcut::apply_shortcuts(&app, &config).map_err(|e| e.to_string())?;
    crate::config::save_shortcuts(&app, &config)
}

/// 录制快捷键时临时取消注册全局快捷键，使按键能传到前端；结束时恢复。
#[tauri::command]
pub fn set_shortcut_recording<R: Runtime>(app: AppHandle<R>, active: bool) {
    shortcut::set_recording_active(&app, active);
}

/// 显示并聚焦主窗口（前端在 OCR 完成后调用）。
#[tauri::command]
pub fn show_main<R: Runtime>(app: AppHandle<R>) {
    window::show_main(&app);
}

/// 普通 AI 对话命令：不使用翻译提示词。
#[tauri::command]
pub async fn chat<R: Runtime>(
    app: AppHandle<R>,
    messages: Vec<ChatMessage>,
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }
    let cfg = config::load_chat_ai(&app);
    translate::openai::chat_stream(&app, &cfg, &messages).await
}

/// 读取固定窗口状态。
#[tauri::command]
pub fn get_pinned() -> bool {
    window::is_pinned()
}

/// 设置固定窗口状态。固定时置顶且失焦不隐藏。
#[tauri::command]
pub fn set_pinned<R: Runtime>(app: AppHandle<R>, pinned: bool) -> Result<bool, String> {
    let Some(w) = window::main_window(&app) else {
        return Err("主窗口不存在".to_string());
    };
    window::set_pinned(&w, pinned).map_err(|e| format!("设置固定窗口失败: {e}"))
}

/// 翻译命令。
///
/// - Google：一次性返回完整译文（String）
/// - OpenAI：流式，通过 `translate-chunk` 事件推送，命令本身返回空串
#[tauri::command]
pub async fn translate<R: Runtime>(
    app: AppHandle<R>,
    req: TranslateRequest,
) -> Result<String, String> {
    let text = req.text.trim();
    if text.is_empty() {
        return Ok(String::new());
    }

    match req.engine {
        Engine::Google => {
            translate::google::translate(text, &req.source, &req.target).await
        }
        Engine::Bing => {
            translate::bing::translate(text, &req.source, &req.target).await
        }
        Engine::Openai => {
            let cfg = config::load_openai(&app);
            translate::openai::translate_stream(&app, &cfg, text, &req.source, &req.target)
                .await?;
            Ok(String::new())
        }
    }
}

/// 截图 OCR 命令：交互式框选 → 系统 OCR → 返回识别文本。
///
/// 用户取消截图时返回空串。在阻塞线程执行（screencapture 与 Vision 均为阻塞调用）。
#[tauri::command]
pub async fn screenshot_ocr() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let Some(path) = screenshot::capture_interactive()? else {
            return Ok(String::new());
        };
        let result = ocr::recognize_file(&path.to_string_lossy());
        // 清理临时文件
        let _ = std::fs::remove_file(&path);
        result
    })
    .await
    .map_err(|e| format!("OCR 任务失败: {e}"))?
}
