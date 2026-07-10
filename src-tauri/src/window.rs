//! 窗口管理：显示/隐藏/切换，以及失焦自动隐藏。
//!
//! 关键策略：窗口**隐藏而非销毁**，保证快捷键秒级呼出。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, WebviewWindow};

pub const MAIN_WINDOW: &str = "main";

static IS_PINNED: AtomicBool = AtomicBool::new(false);

/// 获取主窗口句柄。
pub fn main_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(MAIN_WINDOW)
}

/// 显示并聚焦窗口（居中）。
pub fn show_window<R: tauri::Runtime>(window: &WebviewWindow<R>) {
    let _ = window.center();
    let _ = window.set_always_on_top(is_pinned());
    let _ = window.show();
    let _ = window.set_focus();
}

/// 显示并聚焦主窗口。
pub fn show_main<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(window) = main_window(app) {
        show_window(&window);
    }
}

/// 主窗口当前是否拥有焦点。
#[allow(dead_code)]
pub fn is_main_focused<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    main_window(app)
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}

/// 窗口当前是否可见（查询失败时保守返回 false）。
pub fn is_visible_safe<R: tauri::Runtime>(window: &tauri::Window<R>) -> bool {
    window.is_visible().unwrap_or(false)
}

/// 隐藏主窗口。
pub fn hide_window<R: tauri::Runtime>(window: &WebviewWindow<R>) {
    let _ = window.hide();
}

/// 当前是否处于固定窗口模式。
pub fn is_pinned() -> bool {
    IS_PINNED.load(Ordering::Relaxed)
}

/// 设置固定窗口模式。固定时窗口置顶且失焦不自动隐藏。
pub fn set_pinned<R: tauri::Runtime>(
    window: &WebviewWindow<R>,
    pinned: bool,
) -> tauri::Result<bool> {
    let pinned = commit_pinned_state(&IS_PINNED, pinned, window.set_always_on_top(pinned))?;
    if pinned {
        let _ = window.show();
        let _ = window.set_focus();
    }
    Ok(pinned)
}

fn commit_pinned_state<E>(
    state: &AtomicBool,
    pinned: bool,
    operation: Result<(), E>,
) -> Result<bool, E> {
    operation?;
    state.store(pinned, Ordering::Relaxed);
    Ok(pinned)
}

/// 切换主窗口可见性。返回 true 表示切换后窗口为显示状态。
pub fn toggle_window<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = main_window(app) else {
        return false;
    };
    match window.is_visible() {
        Ok(true) => {
            hide_window(&window);
            false
        }
        _ => {
            show_window(&window);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_state_changes_only_after_window_operation_succeeds() {
        let state = AtomicBool::new(false);

        let failed: Result<bool, &str> = commit_pinned_state(&state, true, Err("failed"));
        assert_eq!(failed, Err("failed"));
        assert!(!state.load(Ordering::Relaxed));

        assert_eq!(
            commit_pinned_state(&state, true, Ok::<(), &str>(())),
            Ok(true)
        );
        assert!(state.load(Ordering::Relaxed));
    }
}
