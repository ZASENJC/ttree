//! TTREE —— 轻量 macOS AI 翻译器后端入口。

mod commands;
mod config;
mod history;
mod keychain;
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
            commands::set_pinned,
            commands::get_appearance_config,
            commands::set_appearance_config,
            commands::load_chat_history,
            commands::load_conversations,
            commands::start_new_conversation,
            commands::append_chat_history,
            commands::clear_chat_history
        ])
        .setup(|app| {
            // 应用毛玻璃磨砂底色 —— Material 3 扁平风格：保留 Popover 通透材质作为窗口底，
            // 圆角与 CSS 的 --radius-window(14px) 对齐，控件一律走实色表面。
            #[cfg(target_os = "macos")]
            if let Some(win) = app.get_webview_window(window::MAIN_WINDOW) {
                let _ = apply_vibrancy(
                    &win,
                    NSVisualEffectMaterial::Popover,
                    Some(NSVisualEffectState::Active),
                    Some(14.0),
                );
            }

            // 恢复上次用户调整后的窗口尺寸；缺失则沿用配置默认值（最小宽度）。
            if let Some(win) = app.get_webview_window(window::MAIN_WINDOW) {
                if let Some((w, h)) = config::load_window_size(app.handle()) {
                    let _ = win.set_size(tauri::LogicalSize::new(w, h));
                }
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
            // 用户调整窗口尺寸后持久化，下次呼出沿用。
            // 仅在窗口可见时记录，避免启动期间窗口隐藏时的初始化尺寸覆盖已保存值。
            if win.label() == window::MAIN_WINDOW && window::is_visible_safe(win) {
                if let WindowEvent::Resized(physical) = event {
                    let scale = win.scale_factor().unwrap_or(1.0);
                    let w = physical.width as f64 / scale;
                    let h = physical.height as f64 / scale;
                    config::save_window_size(win.app_handle(), w, h);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
