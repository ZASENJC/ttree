//! 全局快捷键解析、冲突校验与内存级 O(1) 分发（不读盘）。

use std::str::FromStr;

use anyhow::anyhow;
use global_hotkey::hotkey::{HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;
use ttree_core::ShortcutConfig;

const MAX_SHORTCUT_SPEC_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    Toggle,
    Ocr,
    AiDialog,
    SelectionTranslate,
    SelectionAiDialog,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedShortcuts {
    pub toggle: Option<HotKey>,
    pub ocr: Option<HotKey>,
    pub ai_dialog: Option<HotKey>,
    pub selection_translate: Option<HotKey>,
    pub selection_ai_dialog: Option<HotKey>,
}

impl ParsedShortcuts {
    fn describe(&self, hotkey: HotKey) -> String {
        let action = match self.action_for_id(hotkey.id()) {
            Some(ShortcutAction::Toggle) => "呼出翻译",
            Some(ShortcutAction::Ocr) => "截图 OCR",
            Some(ShortcutAction::AiDialog) => "AI 对话",
            Some(ShortcutAction::SelectionTranslate) => "划词翻译",
            Some(ShortcutAction::SelectionAiDialog) => "划词对话",
            None => "全局快捷键",
        };
        format!("{action}（{hotkey}）")
    }

    /// 划词动作排在对应的纯呼出动作之前，共用同一快捷键时优先走划词逻辑。
    pub fn entries(&self) -> [(ShortcutAction, &Option<HotKey>); 5] {
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

    /// 去重后的待注册热键列表。
    pub fn unique_hotkeys(&self) -> Vec<HotKey> {
        let mut out: Vec<HotKey> = Vec::new();
        for (_, hk) in self.entries() {
            if let Some(hk) = hk {
                if !out.iter().any(|existing| existing.id() == hk.id()) {
                    out.push(*hk);
                }
            }
        }
        out
    }

    /// 根据 `global-hotkey` 的 `id` 在内存中 O(1) 匹配动作。
    pub fn action_for_id(&self, hotkey_id: u32) -> Option<ShortcutAction> {
        self.entries()
            .into_iter()
            .find(|(_, candidate)| candidate.as_ref().is_some_and(|hk| hk.id() == hotkey_id))
            .map(|(action, _)| action)
    }

    fn reject_disallowed_duplicates(&self) -> Result<(), anyhow::Error> {
        let fields: [(&str, &Option<HotKey>); 5] = [
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
                if left.id() == right.id() && !is_allowed_shared_pair(left_label, right_label) {
                    return Err(anyhow!("{left_label}和{right_label}不能相同"));
                }
            }
        }
        Ok(())
    }
}

/// 保留未变的快捷键；新增注册全部成功后才释放旧组合，失败则恢复原注册集合。
pub fn register_shortcuts(
    manager: &GlobalHotKeyManager,
    registered: &mut Vec<HotKey>,
    parsed: &ParsedShortcuts,
) -> Result<(), String> {
    let previous = registered.clone();
    if let Err(error) = sync_hotkeys(manager, registered, &parsed.unique_hotkeys(), parsed) {
        return match sync_hotkeys(manager, registered, &previous, parsed) {
            Ok(()) => Err(error),
            Err(restore_error) => Err(format!("{error}；恢复原快捷键失败: {restore_error}")),
        };
    }
    Ok(())
}

fn sync_hotkeys(
    manager: &GlobalHotKeyManager,
    registered: &mut Vec<HotKey>,
    desired: &[HotKey],
    parsed: &ParsedShortcuts,
) -> Result<(), String> {
    for &hotkey in desired {
        if registered.contains(&hotkey) {
            continue;
        }
        manager.register(hotkey).map_err(|error| {
            let description = parsed.describe(hotkey);
            match error {
                global_hotkey::Error::AlreadyRegistered(_) => format!(
                    "{description}已被系统或其他程序占用，请更换快捷键或退出其他 TTREE 实例。"
                ),
                other => format!("注册{description}失败: {other}"),
            }
        })?;
        registered.push(hotkey);
    }
    for hotkey in registered.clone() {
        if !desired.contains(&hotkey) {
            manager
                .unregister(hotkey)
                .map_err(|error| format!("释放{}失败: {error}", parsed.describe(hotkey)))?;
            registered.retain(|existing| *existing != hotkey);
        }
    }
    Ok(())
}

fn is_allowed_shared_pair(a: &str, b: &str) -> bool {
    const ALLOWED: [(&str, &str); 2] = [
        ("呼出翻译快捷键", "划词翻译快捷键"),
        ("呼出 AI 对话快捷键", "划词 AI 对话快捷键"),
    ];
    ALLOWED
        .iter()
        .any(|&(x, y)| (a == x && b == y) || (a == y && b == x))
}

pub fn parse_shortcut_config(cfg: &ShortcutConfig) -> Result<ParsedShortcuts, anyhow::Error> {
    let parsed = ParsedShortcuts {
        toggle: parse_optional_shortcut("呼出翻译快捷键", &cfg.toggle)?,
        ocr: parse_optional_shortcut("截图 OCR 快捷键", &cfg.ocr)?,
        ai_dialog: parse_optional_shortcut("呼出 AI 对话快捷键", &cfg.ai_dialog)?,
        selection_translate: parse_optional_shortcut("划词翻译快捷键", &cfg.selection_translate)?,
        selection_ai_dialog: parse_optional_shortcut(
            "划词 AI 对话快捷键",
            &cfg.selection_ai_dialog,
        )?,
    };
    parsed.reject_disallowed_duplicates()?;
    Ok(parsed)
}

fn parse_optional_shortcut(label: &str, spec: &str) -> Result<Option<HotKey>, anyhow::Error> {
    if spec.len() > MAX_SHORTCUT_SPEC_BYTES {
        return Err(anyhow!("{label}超出 {MAX_SHORTCUT_SPEC_BYTES} 字节上限"));
    }
    if spec.trim().is_empty() {
        return Ok(None);
    }
    // global-hotkey 已按平台处理 CmdOrCtrl，并支持 Cmd / Option 等别名。
    let hotkey = HotKey::from_str(spec.trim()).map_err(|e| anyhow!("{label}无效: {e}"))?;
    if !hotkey
        .mods
        .intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT)
    {
        return Err(anyhow!(
            "{label}必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"
        ));
    }
    Ok(Some(hotkey))
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::Code;

    fn platform_modifier() -> Modifiers {
        if cfg!(target_os = "macos") {
            Modifiers::SUPER
        } else {
            Modifiers::CONTROL
        }
    }

    #[test]
    fn cmd_or_ctrl_aliases_follow_the_current_platform() {
        for alias in [
            "CmdOrCtrl",
            "CommandOrControl",
            "CommandOrCtrl",
            "CmdOrControl",
            "cmdorctrl",
            "COMMANDORCONTROL",
        ] {
            let hk = parse_optional_shortcut("呼出翻译快捷键", &format!(" {alias} + right "))
                .unwrap()
                .unwrap();
            assert_eq!(hk, HotKey::new(Some(platform_modifier()), Code::ArrowRight));
        }
    }

    #[test]
    fn default_shortcuts_use_the_current_platform_modifier() {
        let parsed = parse_shortcut_config(&ShortcutConfig::default()).unwrap();
        assert_eq!(
            parsed.toggle.unwrap(),
            HotKey::new(Some(platform_modifier() | Modifiers::SHIFT), Code::Space)
        );
        assert_eq!(
            parsed.ocr.unwrap(),
            HotKey::new(Some(platform_modifier() | Modifiers::SHIFT), Code::KeyS)
        );
    }

    #[test]
    fn explicit_command_and_option_keep_their_meaning() {
        assert_eq!(
            parse_optional_shortcut("test", "Cmd+Shift+S")
                .unwrap()
                .unwrap(),
            HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyS)
        );
        assert_eq!(
            parse_optional_shortcut("test", "Option+Space")
                .unwrap()
                .unwrap(),
            HotKey::new(Some(Modifiers::ALT), Code::Space)
        );
        assert!(parse_optional_shortcut("test", "  ").unwrap().is_none());
    }

    #[test]
    fn rejects_conflicts_between_platform_aliases_and_explicit_modifiers() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+S".into(),
            ocr: if cfg!(target_os = "macos") {
                "Command+Shift+S".into()
            } else {
                "Ctrl+Shift+S".into()
            },
            ..ShortcutConfig::default()
        };
        assert!(parse_shortcut_config(&cfg).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_registration_preserves_existing_keys_on_conflict_and_supports_retry() {
        let manager = GlobalHotKeyManager::new().unwrap();
        let blocker = GlobalHotKeyManager::new().unwrap();
        let cfg = ShortcutConfig {
            toggle: "Ctrl+Alt+Shift+F21".into(),
            ocr: "Ctrl+Alt+Shift+F22".into(),
            ..ShortcutConfig::default()
        };
        let original = parse_shortcut_config(&cfg).unwrap();
        let mut registered = Vec::new();
        register_shortcuts(&manager, &mut registered, &original).unwrap();
        let previous = registered.clone();

        let changed = parse_shortcut_config(&ShortcutConfig {
            toggle: "Ctrl+Alt+Shift+F23".into(),
            ocr: "Ctrl+Alt+Shift+F24".into(),
            ..cfg.clone()
        })
        .unwrap();
        let blocked = changed.ocr.unwrap();
        blocker.register(blocked).unwrap();
        let error = register_shortcuts(&manager, &mut registered, &changed).unwrap_err();
        assert!(error.contains("截图 OCR"), "{error}");
        assert!(error.contains("占用"), "{error}");
        assert_eq!(registered, previous);
        for hotkey in &previous {
            assert!(matches!(
                blocker.register(*hotkey),
                Err(global_hotkey::Error::AlreadyRegistered(_))
            ));
        }
        // A key staged before the conflict must be released during rollback.
        blocker.register(changed.toggle.unwrap()).unwrap();
        blocker.unregister(changed.toggle.unwrap()).unwrap();
        blocker.unregister(blocked).unwrap();

        register_shortcuts(&manager, &mut registered, &changed).unwrap();
        assert_eq!(registered, changed.unique_hotkeys());
        for hotkey in &previous {
            blocker.register(*hotkey).unwrap();
            blocker.unregister(*hotkey).unwrap();
        }
        // Repeated saves and swapping actions reuse the existing registrations.
        register_shortcuts(&manager, &mut registered, &changed).unwrap();
        let swapped = ParsedShortcuts {
            toggle: changed.ocr,
            ocr: changed.toggle,
            ..ParsedShortcuts::default()
        };
        register_shortcuts(&manager, &mut registered, &swapped).unwrap();
        assert_eq!(registered.len(), 2);

        let shared = ParsedShortcuts {
            toggle: changed.toggle,
            selection_translate: changed.toggle,
            ..ParsedShortcuts::default()
        };
        register_shortcuts(&manager, &mut registered, &shared).unwrap();
        assert_eq!(registered, shared.unique_hotkeys());
        register_shortcuts(&manager, &mut registered, &ParsedShortcuts::default()).unwrap();
        assert!(registered.is_empty());
        for hotkey in changed.unique_hotkeys() {
            blocker.register(hotkey).unwrap();
            blocker.unregister(hotkey).unwrap();
        }
    }

    #[test]
    fn parses_default_and_shared_shortcuts() {
        let cfg = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+Space".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ai_dialog: "Option+Space".to_string(),
            selection_translate: "CmdOrCtrl+Shift+Space".to_string(),
            selection_ai_dialog: String::new(),
        };
        let parsed = parse_shortcut_config(&cfg).unwrap();
        assert_eq!(parsed.unique_hotkeys().len(), 3);
        let toggle_id = parsed.toggle.unwrap().id();
        assert_eq!(
            parsed.action_for_id(toggle_id),
            Some(ShortcutAction::SelectionTranslate)
        );
    }

    #[test]
    fn rejects_invalid_or_conflicting_shortcuts() {
        let no_mod = ShortcutConfig {
            toggle: "Space".to_string(),
            ..ShortcutConfig::default()
        };
        assert!(parse_shortcut_config(&no_mod).is_err());

        let conflict = ShortcutConfig {
            toggle: "CmdOrCtrl+Shift+S".to_string(),
            ocr: "CmdOrCtrl+Shift+S".to_string(),
            ..ShortcutConfig::default()
        };
        assert!(parse_shortcut_config(&conflict).is_err());
    }
}
