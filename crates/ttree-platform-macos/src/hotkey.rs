//! 全局快捷键解析、冲突校验与内存级 O(1) 分发（不读盘）。

use std::str::FromStr;

use anyhow::anyhow;
use global_hotkey::hotkey::{HotKey, Modifiers};
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

fn is_allowed_shared_pair(a: &str, b: &str) -> bool {
    const ALLOWED: [(&str, &str); 2] = [
        ("呼出翻译快捷键", "划词翻译快捷键"),
        ("呼出 AI 对话快捷键", "划词 AI 对话快捷键"),
    ];
    ALLOWED
        .iter()
        .any(|&(x, y)| (a == x && b == y) || (a == y && b == x))
}

/// 将 Tauri 风格的 `CmdOrCtrl+Shift+Space` / `Option+Space` 规范化为 `global-hotkey` 支持的格式。
fn normalize_spec(spec: &str) -> String {
    spec.trim()
        .split('+')
        .map(|part| match part.trim() {
            "CmdOrCtrl" | "CommandOrControl" | "Cmd" | "Command" => "Super",
            "Option" => "Alt",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
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
    let normalized = normalize_spec(spec);
    let hotkey = HotKey::from_str(&normalized).map_err(|e| anyhow!("{label}无效: {e}"))?;
    if !hotkey
        .mods
        .intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT)
    {
        return Err(anyhow!("{label}必须包含 Cmd/Ctrl/Option 中的至少一个修饰键"));
    }
    Ok(Some(hotkey))
}

#[cfg(test)]
mod tests {
    use super::*;

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
