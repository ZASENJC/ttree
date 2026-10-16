//! Read-only native edits that record key combinations, never shortcut syntax.
use super::*;
use global_hotkey::hotkey::{Code, Modifiers};
use std::collections::HashMap;
use ttree_platform::shortcut_recorder::{CaptureResult, ShortcutRecorder};

thread_local! {
    static RECORDERS: RefCell<HashMap<usize, ShortcutRecorder>> = RefCell::new(HashMap::new());
}

#[link(name = "user32")]
extern "system" {
    fn GetDlgCtrlID(hwnd: HWND) -> i32;
    fn GetKeyState(key: i32) -> i16;
    fn GetAncestor(hwnd: HWND, flags: u32) -> HWND;
    fn GetDlgItem(parent: HWND, id: i32) -> HWND;
}

pub(super) unsafe fn is_field(hwnd: HWND) -> bool {
    !hwnd.is_null()
        && matches!(
            GetDlgCtrlID(hwnd) as usize,
            ID_EDIT_HK_TOGGLE
                | ID_EDIT_HK_OCR
                | ID_EDIT_HK_CHAT
                | ID_EDIT_HK_SEL_TRANS
                | ID_EDIT_HK_SEL_CHAT
        )
}

pub(super) unsafe fn init(hwnd: HWND) {
    RECORDERS.with(|recorders| {
        recorders
            .borrow_mut()
            .insert(hwnd as usize, ShortcutRecorder::default())
    });
    SendMessageW(hwnd, 0x00cf, 1, 0); // EM_SETREADONLY: blocks IME and context-menu edits too.
    SendMessageW(
        hwnd,
        0x1501,
        1,
        wide_null("点击后按组合键").as_ptr() as LPARAM,
    );
}

pub(super) unsafe fn set_value(hwnd: HWND, spec: &str) {
    let display = RECORDERS.with(|recorders| {
        let mut recorders = recorders.borrow_mut();
        let recorder = recorders.entry(hwnd as usize).or_default();
        recorder.set(spec);
        recorder.display()
    });
    set_hwnd_text(hwnd, &display);
}

pub(super) fn value(hwnd: HWND) -> String {
    RECORDERS.with(|recorders| {
        recorders
            .borrow()
            .get(&(hwnd as usize))
            .map(|recorder| recorder.value().to_owned())
            .unwrap_or_default()
    })
}

unsafe fn modifiers() -> Modifiers {
    let mut mods = Modifiers::empty();
    for (vk, modifier) in [
        (0x11, Modifiers::CONTROL),
        (0x12, Modifiers::ALT),
        (0x10, Modifiers::SHIFT),
    ] {
        if GetKeyState(vk) < 0 {
            mods |= modifier;
        }
    }
    if GetKeyState(0x5b) < 0 || GetKeyState(0x5c) < 0 {
        mods |= Modifiers::SUPER;
    }
    mods
}

fn key(vk: usize) -> Option<Code> {
    let token = match vk {
        0x30..=0x39 | 0x41..=0x5a => char::from_u32(vk as u32)?.to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6f),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        _ => match vk {
            0x08 => "Backspace",
            0x09 => "Tab",
            0x0d => "Enter",
            0x1b => "Escape",
            0x20 => "Space",
            0x21 => "PageUp",
            0x22 => "PageDown",
            0x23 => "End",
            0x24 => "Home",
            0x25 => "Left",
            0x26 => "Up",
            0x27 => "Right",
            0x28 => "Down",
            0x2c => "PrintScreen",
            0x2d => "Insert",
            0x2e => "Delete",
            0x6a => "NumpadMultiply",
            0x6b => "NumpadAdd",
            0x6d => "NumpadSubtract",
            0x6e => "NumpadDecimal",
            0x6f => "NumpadDivide",
            0xba => "Semicolon",
            0xbb => "Equal",
            0xbc => "Comma",
            0xbd => "Minus",
            0xbe => "Period",
            0xbf => "Slash",
            0xc0 => "Backquote",
            0xdb => "BracketLeft",
            0xdc => "Backslash",
            0xdd => "BracketRight",
            0xde => "Quote",
            _ => return None,
        }
        .to_owned(),
    };
    token.parse::<HotKey>().ok().map(|hotkey| hotkey.key)
}

pub(super) unsafe fn handle(hwnd: HWND, msg: u32, w: WPARAM, _l: LPARAM) -> Option<LRESULT> {
    if !is_field(hwnd) {
        return None;
    }
    match msg {
        WM_SETFOCUS => {
            RECORDERS.with(|recorders| {
                recorders
                    .borrow_mut()
                    .entry(hwnd as usize)
                    .or_default()
                    .begin();
            });
            with_state(|state| state.set_shortcut_recording(true));
            SendMessageW(hwnd, EM_SETSEL, 0, -1);
        }
        WM_KILLFOCUS => {
            if !is_field(w as HWND) {
                with_state(|state| state.set_shortcut_recording(false));
            }
        }
        0x0087 => return Some(4), // WM_GETDLGCODE: DLGC_WANTALLKEYS
        WM_KEYDOWN | 0x0104 => {
            // WM_SYSKEYDOWN includes Alt combinations.
            if let Some(key) = key(w) {
                let (result, display) = RECORDERS.with(|recorders| {
                    let mut recorders = recorders.borrow_mut();
                    let recorder = recorders.entry(hwnd as usize).or_default();
                    let result = recorder.capture(key, modifiers());
                    (result, recorder.display())
                });
                match result {
                    CaptureResult::Changed => {
                        set_hwnd_text(hwnd, &display);
                        SendMessageW(hwnd, EM_SETSEL, 0, -1);
                    }
                    CaptureResult::Cancelled => {
                        set_hwnd_text(hwnd, &display);
                        SetFocus(GetAncestor(hwnd, 2)); // GA_ROOT; Esc leaves settings open.
                    }
                    CaptureResult::Navigate { backwards } => {
                        // Layout changes the child Z-order, so follow the visible field order.
                        let fields = [
                            ID_EDIT_HK_TOGGLE,
                            ID_EDIT_HK_OCR,
                            ID_EDIT_HK_CHAT,
                            ID_EDIT_HK_SEL_TRANS,
                            ID_EDIT_HK_SEL_CHAT,
                            ID_BTN_HK_RESET,
                        ];
                        let index = fields
                            .iter()
                            .position(|&id| id == GetDlgCtrlID(hwnd) as usize)
                            .unwrap_or(0);
                        let offset = if backwards { fields.len() - 1 } else { 1 };
                        let next = GetDlgItem(
                            GetParent(hwnd),
                            fields[(index + offset) % fields.len()] as i32,
                        );
                        if !next.is_null() {
                            SetFocus(next);
                        }
                    }
                    CaptureResult::Ignored => {}
                }
            }
            return Some(0);
        }
        WM_CHAR | 0x0106 | 0x0300..=0x0304 | 0x007b | 0x010d..=0x010f => return Some(0),
        0x0082 => {
            RECORDERS.with(|recorders| recorders.borrow_mut().remove(&(hwnd as usize)));
        }
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_physical_keys_and_leaves_modifiers_unrecorded() {
        assert_eq!(key(0x53), Some(Code::KeyS));
        assert_eq!(key(0x27), Some(Code::ArrowRight));
        assert_eq!(key(0x87), Some(Code::F24));
        assert_eq!(key(0x6b), Some(Code::NumpadAdd));
        assert_eq!(key(0xbb), Some(Code::Equal));
        assert_eq!(key(0x11), None);
        assert_eq!(key(0x5b), None);
    }
}
