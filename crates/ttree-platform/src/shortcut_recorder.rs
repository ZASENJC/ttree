//! Keyboard recording keeps persistent shortcut syntax separate from native labels.
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

#[derive(Debug, PartialEq, Eq)]
pub enum CaptureResult {
    Changed,
    Cancelled,
    Navigate { backwards: bool },
    Ignored,
}

#[derive(Default)]
pub struct ShortcutRecorder {
    value: String,
    original: String,
}

impl ShortcutRecorder {
    pub fn set(&mut self, value: &str) {
        self.value = value.to_owned();
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn begin(&mut self) {
        self.original = self.value.clone();
    }

    pub fn display(&self) -> String {
        let Ok(hotkey) = self.value.parse::<HotKey>() else {
            return self.value.clone();
        };
        let mut parts = Vec::new();
        if hotkey.mods.contains(Modifiers::CONTROL) {
            parts.push("Ctrl".to_owned());
        }
        if hotkey.mods.contains(Modifiers::ALT) {
            parts.push(
                if cfg!(target_os = "macos") {
                    "Option"
                } else {
                    "Alt"
                }
                .into(),
            );
        }
        if hotkey.mods.contains(Modifiers::SHIFT) {
            parts.push("Shift".to_owned());
        }
        if hotkey.mods.contains(Modifiers::SUPER) {
            parts.push(
                if cfg!(target_os = "macos") {
                    "Cmd"
                } else {
                    "Win"
                }
                .into(),
            );
        }
        let code = hotkey.key.to_string();
        let label = code
            .strip_prefix("Key")
            .or_else(|| code.strip_prefix("Digit"))
            .or_else(|| code.strip_prefix("Arrow"))
            .unwrap_or(&code);
        parts.push(label.to_owned());
        parts.join(" + ")
    }

    pub fn capture(&mut self, key: Code, modifiers: Modifiers) -> CaptureResult {
        let command = modifiers.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER);
        if !command {
            match key {
                Code::Escape if modifiers.is_empty() => {
                    self.value = self.original.clone();
                    return CaptureResult::Cancelled;
                }
                Code::Backspace | Code::Delete if modifiers.is_empty() => {
                    self.value.clear();
                    return CaptureResult::Changed;
                }
                Code::Tab => {
                    return CaptureResult::Navigate {
                        backwards: modifiers.contains(Modifiers::SHIFT),
                    };
                }
                _ => return CaptureResult::Ignored,
            }
        }
        // Modifier-only events must never create a shortcut.
        if matches!(
            key,
            Code::ControlLeft
                | Code::ControlRight
                | Code::AltLeft
                | Code::AltRight
                | Code::ShiftLeft
                | Code::ShiftRight
                | Code::MetaLeft
                | Code::MetaRight
        ) {
            return CaptureResult::Ignored;
        }
        let modifiers =
            modifiers & (Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER | Modifiers::SHIFT);
        self.value = HotKey::new(Some(modifiers), key).into_string();
        CaptureResult::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_combinations_as_round_trippable_config_and_readable_labels() {
        let mut recorder = ShortcutRecorder::default();
        recorder.begin();
        assert_eq!(
            recorder.capture(Code::KeyS, Modifiers::CONTROL | Modifiers::SHIFT),
            CaptureResult::Changed
        );
        let hk: HotKey = recorder.value().parse().unwrap();
        assert_eq!(
            hk,
            HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS)
        );
        assert_eq!(recorder.display(), "Ctrl + Shift + S");
        recorder.capture(Code::ArrowRight, Modifiers::CONTROL);
        assert_eq!(recorder.display(), "Ctrl + Right");
        recorder.capture(Code::NumpadAdd, Modifiers::ALT);
        assert_eq!(
            recorder.value().parse::<HotKey>().unwrap().key,
            Code::NumpadAdd
        );
    }

    #[test]
    fn ignores_plain_text_shift_only_and_modifier_only_keys() {
        let mut recorder = ShortcutRecorder::default();
        recorder.set("Ctrl+Right");
        for (key, modifiers) in [
            (Code::KeyA, Modifiers::empty()),
            (Code::KeyA, Modifiers::SHIFT),
            (Code::ControlLeft, Modifiers::CONTROL),
        ] {
            assert_eq!(recorder.capture(key, modifiers), CaptureResult::Ignored);
            assert_eq!(recorder.value(), "Ctrl+Right");
        }
    }

    #[test]
    fn supports_clear_cancel_and_keyboard_navigation_without_recording_them() {
        let mut recorder = ShortcutRecorder::default();
        recorder.set("Ctrl+Right");
        recorder.begin();
        assert_eq!(
            recorder.capture(Code::Backspace, Modifiers::empty()),
            CaptureResult::Changed
        );
        assert_eq!(recorder.value(), "");
        assert_eq!(
            recorder.capture(Code::Escape, Modifiers::empty()),
            CaptureResult::Cancelled
        );
        assert_eq!(recorder.value(), "Ctrl+Right");
        assert_eq!(
            recorder.capture(Code::Tab, Modifiers::empty()),
            CaptureResult::Navigate { backwards: false }
        );
        assert_eq!(
            recorder.capture(Code::Tab, Modifiers::SHIFT),
            CaptureResult::Navigate { backwards: true }
        );
        assert_eq!(
            recorder.capture(Code::Delete, Modifiers::CONTROL),
            CaptureResult::Changed
        );
        assert_eq!(
            recorder.value().parse::<HotKey>().unwrap().key,
            Code::Delete
        );
    }

    #[test]
    fn displays_existing_platform_aliases_without_rewriting_the_saved_value() {
        let mut recorder = ShortcutRecorder::default();
        recorder.set("CmdOrCtrl+Shift+Space");
        assert_eq!(recorder.value(), "CmdOrCtrl+Shift+Space");
        assert_eq!(
            recorder.display(),
            if cfg!(target_os = "macos") {
                "Shift + Cmd + Space"
            } else {
                "Ctrl + Shift + Space"
            }
        );
    }
}
