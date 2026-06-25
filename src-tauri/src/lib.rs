//! ttree —— 轻量 macOS AI 翻译器后端入口。

mod commands;
mod config;
mod ocr;
mod screenshot;
mod selection;
mod shortcut;
mod tray;
mod translate;
mod window;

use tauri::{Manager, WindowEvent};

#[cfg(target_os = "macos")]
use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(shortcut::build_plugin())
        .invoke_handler(tauri::generate_handler![
            commands::translate,
            commands::chat,
            commands::screenshot_ocr,
            commands::show_main,
            commands::get_openai_config,
            commands::set_openai_config,
            commands::get_chat_ai_config,
            commands::set_chat_ai_config,
            commands::get_shortcut_config,
            commands::set_shortcut_config,
            commands::set_shortcut_recording,
            commands::get_pinned,
            commands::set_pinned
        ])
        .setup(|app| {
            // 应用毛玻璃磨砂效果
            #[cfg(target_os = "macos")]
            if let Some(win) = app.get_webview_window(window::MAIN_WINDOW) {
                let _ = apply_vibrancy(
                    &win,
                    NSVisualEffectMaterial::HudWindow,
                    Some(NSVisualEffectState::Active),
                    Some(16.0),
                );
            }

            // 注册持久化快捷键配置
            shortcut::register_saved(app.handle())?;

            // 构建系统托盘
            tray::setup_tray(app.handle())?;

            Ok(())
        })
        .on_window_event(|win, event| {
            // 失焦自动隐藏（仅主窗口）。固定窗口时不隐藏。
            if win.label() == window::MAIN_WINDOW && !window::is_pinned() {
                if let WindowEvent::Focused(false) = event {
                    let _ = win.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
