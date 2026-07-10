//! 全局快捷键注册与分发。
//!
//! - 呼出快捷键：toggle 主窗口可见性
//! - 截图 OCR 快捷键：广播 `trigger-ocr`，前端调用截图/OCR/翻译流程
//! - AI 对话快捷键：呼出 AI 对话窗口
//! - 划词翻译快捷键：捕获选中文本并自动翻译
//! - 划词 AI 对话快捷键：捕获选中文本并发送给 AI
//!
//! 所有快捷键均可选；空字符串表示未启用。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{
    Builder as ShortcutBuilder, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

use crate::config::{self, ShortcutConfig};
use crate::selection;
use crate::window;

static IS_RECORDING_SHORTCUT: AtomicBool = AtomicBool::new(false);
const MAX_SHORTCUT_SPEC_BYTES: usize = 128;

#[derive(Debug, Clone, Default)]
struct ParsedShortcuts {
    toggle: Option<Shortcut>,
    ocr: Option<Shortcut>,
    ai_dialog: Option<Shortcut>,
    selection_translate: Option<Shortcut>,
    selection_ai_dialog: Option<Shortcut>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutAction {
    Toggle,
    Ocr,
    AiDialog,
    SelectionTranslate,
    SelectionAiDialog,
}

#[derive(Debug, Clone, Serialize)]
struct SelectedTextPayload {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct AiDialogPayload {
    text: Option<String>,
}

impl ParsedShortcuts {
    /// 分发优先级顺序：划词动作排在对应的纯呼出动作之前，
    /// 这样当两者共用同一快捷键时，按更丰富的划词行为执行。
    fn entries(&self) -> [(ShortcutAction, &Option<Shortcut>); 5] {
        [
            (
                ShortcutAction::SelectionTranslate,
                &self.selection_translate,
            ),
            (ShortcutAction::SelectionAiDialog, &self.selection_ai_dialog),
            (ShortcutAction::Toggle, &self.toggle),
            (ShortcutAction::Ocr, &self.ocr),
            (ShortcutAction::AiDialog, &self.ai_dialog),
        ]
    }

    /// 需要注册的快捷键集合，去重（共用同一键的只注册一次）。
    fn shortcuts(&self) -> Vec<Shortcut> {
        let mut out: Vec<Shortcut> = Vec::new();
        for (_, shortcut) in self.entries() {
            if let Some(shortcut) = shortcut {
                if !out.contains(shortcut) {
                    out.push(*shortcut);
                }
            }
        }
        out
    }

    fn action_for(&self, shortcut: &Shortcut) -> Option<ShortcutAction> {
        self.entries()
            .into_iter()
            .find(|(_, candidate)| candidate.as_ref() == Some(shortcut))
            .map(|(action, _)| action)
    }

    /// 校验快捷键不冲突。允许两组共用同一键：
    /// 呼出翻译↔划词翻译、呼出 AI 对话↔划词 AI 对话；其余两两不得相同。
    fn reject_disallowed_duplicates(&self) -> Result<(), anyhow::Error> {
        let fields: [(&str, &Option<Shortcut>); 5] = [
            ("呼出翻译快捷键", &self.toggle),
            ("截图 OCR 快捷键", &self.ocr),
            ("呼出 AI 对话快捷键", &self.ai_dialog),
            ("划词翻译快捷键", &self.selection_translate),
            ("划词 AI 对话快捷键", &self.selection_ai_dialog),
        ];

        for (i, (left_label, left)) in fields.iter().enumerate() {
            let Some(left) = left else { continue };
            for (right_label, right) in fields.iter().skip(i + 1) {
                let Some(right) = right else { continue };
                if left == right && !is_allowed_shared_pair(left_label, right_label) {
                    return Err(anyhow::anyhow!("{left_label}和{right_label}不能相同"));
                }
            }
        }

        Ok(())
    }
}

/// 这两组快捷键允许共用同一键（划词动作叠加在纯呼出动作上）。
fn is_allowed_shared_pair(a: &str, b: &str) -> bool {
    const ALLOWED: [(&str, &str); 2] = [
        ("呼出翻译快捷键", "划词翻译快捷键"),
        ("呼出 AI 对话快捷键", "划词 AI 对话快捷键"),
    ];
    ALLOWED
        .iter()
        .any(|&(x, y)| (a == x && b == y) || (a == y && b == x))
}

/// 进入/退出快捷键录制。
///
/// 录制期间取消注册所有全局快捷键，否则与已注册键相同的按键会被 OS 级全局热键
/// 拦截而无法作为普通 keydown 事件传到前端（表现为“按相同键没反应”）。
/// 退出录制时按已保存配置重新注册。
pub fn set_recording_active<R: tauri::Runtime>(
    app: &AppHandle<R>,
    is_active: bool,
) -> Result<(), String> {
    let operation = if is_active {
        app.global_shortcut()
            .unregister_all()
            .map_err(|e| format!("暂停全局快捷键失败: {e}"))
    } else {
        let saved = config::load_shortcuts(app);
        apply_shortcuts(app, &saved).map_err(|e| format!("恢复全局快捷键失败: {e}"))
    };
    transition_recording_state(&IS_RECORDING_SHORTCUT, is_active, operation)
}

fn transition_recording_state<E>(
    state: &AtomicBool,
    is_active: bool,
    operation: Result<(), E>,
) -> Result<(), E> {
    operation?;
    state.store(is_active, Ordering::SeqCst);
    Ok(())
}

/// 构建 global-shortcut 插件，带统一处理器。
pub fn build_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    ShortcutBuilder::<R>::new()
        .with_handler(move |app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            handle_shortcut(app, shortcut);
        })
        .build()
}

fn handle_shortcut<R: tauri::Runtime>(app: &AppHandle<R>, shortcut: &Shortcut) {
    if IS_RECORDING_SHORTCUT.load(Ordering::SeqCst) {
        return;
    }

    let cfg = config::load_shortcuts(app);
    let Ok(shortcuts) = parse_shortcut_config(&cfg) else {
        return;
    };

    match shortcuts.action_for(shortcut) {
        Some(ShortcutAction::Toggle) => {
            // 仅在切换为显示时复位到翻译页。
            if window::toggle_window(app) {
                let _ = app.emit("trigger-translate", ());
            }
        }
        Some(ShortcutAction::Ocr) => {
            let _ = app.emit("trigger-ocr", ());
        }
        Some(ShortcutAction::AiDialog) => handle_ai_dialog_shortcut(app),
        Some(ShortcutAction::SelectionTranslate) => handle_selection_translate_shortcut(app),
        Some(ShortcutAction::SelectionAiDialog) => handle_selection_ai_dialog_shortcut(app),
        None => {}
    }
}

/// 划词翻译：有选中文本则翻译，否则仅 toggle 窗口。
fn handle_selection_translate_shortcut<R: tauri::Runtime>(app: &AppHandle<R>) {
    match selection::capture_selected_text() {
        Ok(Some(text)) => {
            window::show_main(app);
            let _ = app.emit("trigger-selected-translate", SelectedTextPayload { text });
        }
        Ok(None) | Err(_) => {
            window::toggle_window(app);
        }
    }
}

/// 呼出 AI 对话窗口（不捕获划词）。
fn handle_ai_dialog_shortcut<R: tauri::Runtime>(app: &AppHandle<R>) {
    window::show_main(app);
    let _ = app.emit("trigger-ai-dialog", AiDialogPayload { text: None });
}

/// 划词 AI 对话：有选中文本则连同原文发送给 AI，否则仅呼出对话窗口。
fn handle_selection_ai_dialog_shortcut<R: tauri::Runtime>(app: &AppHandle<R>) {
    let text = selection::capture_selected_text().ok().flatten();
    window::show_main(app);
    let _ = app.emit("trigger-ai-dialog", AiDialogPayload { text });
}

/// 注册持久化配置中的快捷键。在 setup 中调用。
pub fn register_saved<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let saved = config::load_shortcuts(app);
    if parse_shortcut_config(&saved).is_ok() && apply_shortcuts(app, &saved).is_ok() {
        return Ok(());
    }

    let defaults = ShortcutConfig::default();
    if apply_shortcuts(app, &defaults).is_ok() {
        let _ = config::save_shortcuts(app, &defaults);
    }

    Ok(())
}

/// 验证并应用快捷键配置。保存设置时调用，立即生效。
pub fn apply_shortcuts<R: tauri::Runtime>(
    app: &AppHandle<R>,
    cfg: &ShortcutConfig,
) -> Result<(), anyhow::Error> {
    let shortcuts = parse_shortcut_config(cfg)?;

    let previous = parse_shortcut_config(&config::load_shortcuts(app)).ok();
    let gs = app.global_shortcut();

    gs.unregister_all()?;

    for shortcut in shortcuts.shortcuts() {
        if let Err(error) = gs.register(shortcut) {
            let _ = gs.unregister_all();
            restore_shortcuts(app, previous);
            return Err(error.into());
        }
    }

    Ok(())
}

fn restore_shortcuts<R: tauri::Runtime>(app: &AppHandle<R>, previous: Option<ParsedShortcuts>) {
    let Some(previous) = previous else {
        return;
    };

    let gs = app.global_shortcut();
    for shortcut in previous.shortcuts() {
        let _ = gs.register(shortcut);
    }
}

fn parse_shortcut_config(cfg: &ShortcutConfig) -> Result<ParsedShortcuts, anyhow::Error> {
    let toggle = parse_optional_shortcut("呼出翻译快捷键", &cfg.toggle)?;
    let ocr = parse_optional_shortcut("截图 OCR 快捷键", &cfg.ocr)?;
    let ai_dialog = parse_optional_shortcut("呼出 AI 对话快捷键", &cfg.ai_dialog)?;
    let selection_translate = parse_optional_shortcut("划词翻译快捷键", &cfg.selection_translate)?;
    let selection_ai_dialog =
        parse_optional_shortcut("划词 AI 对话快捷键", &cfg.selection_ai_dialog)?;

    let parsed = ParsedShortcuts {
        toggle,
        ocr,
        ai_dialog,
        selection_translate,
        selection_ai_dialog,
    };
    parsed.reject_disallowed_duplicates()?;
    Ok(parsed)
}

fn parse_required_shortcut(label: &str, spec: &str) -> Result<Shortcut, anyhow::Error> {
    let shortcut = spec
        .trim()
        .parse::<Shortcut>()
        .map_err(|e| anyhow::anyhow!("{label}无效: {e}"))?;

    if !shortcut
        .mods
        .intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT)
    {
        return Err(anyhow::anyhow!(
            "{label}必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"
        ));
    }

    Ok(shortcut)
}

fn parse_optional_shortcut(label: &str, spec: &str) -> Result<Option<Shortcut>, anyhow::Error> {
    if spec.len() > MAX_SHORTCUT_SPEC_BYTES {
        return Err(anyhow::anyhow!(
            "{label}超出 {MAX_SHORTCUT_SPEC_BYTES} 字节上限"
        ));
    }
    if spec.trim().is_empty() {
        return Ok(None);
    }

    parse_required_shortcut(label, spec).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut_config(toggle: &str, ocr: &str, ai_dialog: &str) -> ShortcutConfig {
        ShortcutConfig {
            toggle: toggle.to_string(),
            ocr: ocr.to_string(),
            ai_dialog: ai_dialog.to_string(),
            selection_translate: String::new(),
            selection_ai_dialog: String::new(),
        }
    }

    #[test]
    fn accepts_default_shortcuts_with_unset_ai_dialog() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+S",
            "",
        ));

        assert!(result.is_ok());
        assert!(result.unwrap().ai_dialog.is_none());
    }

    #[test]
    fn accepts_configured_ai_dialog_shortcut() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+S",
            "CmdOrCtrl+Shift+A",
        ));

        assert!(result.is_ok());
        assert!(result.unwrap().ai_dialog.is_some());
    }

    #[test]
    fn accepts_all_empty_shortcuts() {
        let result = parse_shortcut_config(&shortcut_config("", "", ""));

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(parsed.toggle.is_none());
        assert!(parsed.ocr.is_none());
    }

    #[test]
    fn accepts_selection_shortcuts() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: "CmdOrCtrl+Shift+T".to_string(),
            selection_ai_dialog: "CmdOrCtrl+Shift+D".to_string(),
        };
        let result = parse_shortcut_config(&cfg);

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(parsed.selection_translate.is_some());
        assert!(parsed.selection_ai_dialog.is_some());
    }

    #[test]
    fn accepts_option_space_with_modifier() {
        let result =
            parse_shortcut_config(&shortcut_config("Option+Space", "CmdOrCtrl+Shift+S", ""));

        assert!(result.is_ok());
    }

    #[test]
    fn treats_whitespace_ai_dialog_as_unset() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+S",
            "   ",
        ));

        assert!(result.is_ok());
        assert!(result.unwrap().ai_dialog.is_none());
    }

    #[test]
    fn rejects_toggle_without_modifier() {
        let result = parse_shortcut_config(&shortcut_config("Space", "CmdOrCtrl+Shift+S", ""));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("呼出翻译快捷键必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"));
    }

    #[test]
    fn rejects_ocr_without_modifier() {
        let result = parse_shortcut_config(&shortcut_config("CmdOrCtrl+Shift+Space", "S", ""));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("截图 OCR 快捷键必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"));
    }

    #[test]
    fn rejects_ai_dialog_without_modifier() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+S",
            "A",
        ));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("呼出 AI 对话快捷键必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"));
    }

    #[test]
    fn rejects_selection_translate_without_modifier() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: "T".to_string(),
            selection_ai_dialog: String::new(),
        };
        let result = parse_shortcut_config(&cfg);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("划词翻译快捷键必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"));
    }

    #[test]
    fn rejects_shift_only_shortcut() {
        let result = parse_shortcut_config(&shortcut_config("Shift+S", "CmdOrCtrl+Shift+S", ""));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("呼出翻译快捷键必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"));
    }

    #[test]
    fn rejects_modifier_only_shortcut() {
        let result = parse_shortcut_config(&shortcut_config("Shift", "CmdOrCtrl+Shift+S", ""));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("呼出翻译快捷键无效"));
    }

    #[test]
    fn rejects_duplicate_toggle_and_ocr_shortcuts() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+Space",
            "",
        ));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("呼出翻译快捷键和截图 OCR 快捷键不能相同"));
    }

    #[test]
    fn rejects_duplicate_toggle_and_ai_dialog_shortcuts() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+S",
            "CmdOrCtrl+Shift+Space",
        ));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("呼出翻译快捷键和呼出 AI 对话快捷键不能相同"));
    }

    #[test]
    fn rejects_duplicate_ocr_and_ai_dialog_shortcuts() {
        let result = parse_shortcut_config(&shortcut_config(
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Shift+S",
            "CmdOrCtrl+Shift+S",
        ));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("截图 OCR 快捷键和呼出 AI 对话快捷键不能相同"));
    }

    #[test]
    fn rejects_duplicate_selection_shortcuts() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: "CmdOrCtrl+Shift+T".to_string(),
            selection_ai_dialog: "CmdOrCtrl+Shift+T".to_string(),
        };
        let result = parse_shortcut_config(&cfg);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("划词翻译快捷键和划词 AI 对话快捷键不能相同"));
    }

    #[test]
    fn allows_shared_toggle_and_selection_translate() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: "CmdOrCtrl+Shift+Space".to_string(),
            selection_ai_dialog: String::new(),
        };
        let result = parse_shortcut_config(&cfg);

        assert!(result.is_ok());
        // 共用同一键时只注册一次。
        assert_eq!(result.unwrap().shortcuts().len(), 2);
    }

    #[test]
    fn allows_shared_ai_dialog_and_selection_ai_dialog() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: "CmdOrCtrl+Shift+A".to_string(),
            selection_translate: String::new(),
            selection_ai_dialog: "CmdOrCtrl+Shift+A".to_string(),
        };
        let result = parse_shortcut_config(&cfg);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().shortcuts().len(), 3);
    }

    #[test]
    fn shared_translate_key_dispatches_to_selection_action() {
        // 呼出翻译与划词翻译共用同一键时，划词动作优先。
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: "CmdOrCtrl+Shift+Space".to_string(),
            selection_ai_dialog: String::new(),
        };
        let shortcuts = parse_shortcut_config(&cfg).unwrap();

        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+Space".parse().unwrap()),
            Some(ShortcutAction::SelectionTranslate)
        );
    }

    #[test]
    fn shared_ai_key_dispatches_to_selection_action() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: "CmdOrCtrl+Shift+A".to_string(),
            selection_translate: String::new(),
            selection_ai_dialog: "CmdOrCtrl+Shift+A".to_string(),
        };
        let shortcuts = parse_shortcut_config(&cfg).unwrap();

        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+A".parse().unwrap()),
            Some(ShortcutAction::SelectionAiDialog)
        );
    }

    #[test]
    fn rejects_cross_group_shared_key() {
        // 呼出翻译与划词 AI 对话不属于允许共用的组合。
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: String::new(),
            selection_translate: String::new(),
            selection_ai_dialog: "CmdOrCtrl+Shift+Space".to_string(),
        };
        let result = parse_shortcut_config(&cfg);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不能相同"));
    }

    #[test]
    fn resolves_known_shortcut_actions() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: "CmdOrCtrl+Shift+A".to_string(),
            selection_translate: "CmdOrCtrl+Shift+T".to_string(),
            selection_ai_dialog: "CmdOrCtrl+Shift+D".to_string(),
        };
        let shortcuts = parse_shortcut_config(&cfg).unwrap();

        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+Space".parse().unwrap()),
            Some(ShortcutAction::Toggle)
        );
        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+S".parse().unwrap()),
            Some(ShortcutAction::Ocr)
        );
        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+A".parse().unwrap()),
            Some(ShortcutAction::AiDialog)
        );
        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+T".parse().unwrap()),
            Some(ShortcutAction::SelectionTranslate)
        );
        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+D".parse().unwrap()),
            Some(ShortcutAction::SelectionAiDialog)
        );
        assert_eq!(
            shortcuts.action_for(&"CmdOrCtrl+Shift+X".parse().unwrap()),
            None
        );
    }

    #[test]
    fn recording_state_changes_only_after_registration_operation_succeeds() {
        let state = AtomicBool::new(false);

        let failed: Result<(), &str> = transition_recording_state(&state, true, Err("failed"));
        assert_eq!(failed, Err("failed"));
        assert!(!state.load(Ordering::SeqCst));

        transition_recording_state(&state, true, Ok::<(), &str>(())).unwrap();
        assert!(state.load(Ordering::SeqCst));
    }

    #[test]
    fn rejects_oversized_shortcut_specs_before_parsing() {
        let error = parse_optional_shortcut("测试快捷键", &"x".repeat(129)).unwrap_err();

        assert!(error.to_string().contains("128 字节"));
    }
}
