//! macOS 选中文本捕获：优先走 Accessibility API (`AXSelectedText`)，
//! 仅当辅助功能未授权或焦点控件不支持 AX 属性时才回退到临时剪贴板复制。

use anyhow::Result;

pub fn normalize_selected_text(text: impl AsRef<str>) -> Option<String> {
    let trimmed = text.as_ref().trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ptr;
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{anyhow, Result};
    use core_foundation::base::{Boolean, CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use objc2_app_kit::{NSPasteboard, NSPasteboardType, NSPasteboardTypeString};
    use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation};
    use objc2_foundation::{NSData, NSInteger, NSString};

    use super::normalize_selected_text;

    const AX_ERROR_SUCCESS: i32 = 0;
    const AX_ERROR_NO_VALUE: i32 = -25212;
    const COPY_KEY_CODE: u16 = 8;
    const COPY_TIMEOUT: Duration = Duration::from_millis(320);
    const COPY_POLL_INTERVAL: Duration = Duration::from_millis(16);

    #[repr(C)]
    struct __AXUIElement {
        _private: [u8; 0],
    }

    type AXUIElementRef = *const __AXUIElement;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> Boolean;
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum AxOutcome {
        Selected(String),
        EmptySelection,
        Unavailable,
    }

    #[derive(Debug, Clone)]
    struct PasteboardSnapshot {
        entries: Vec<PasteboardEntry>,
    }

    #[derive(Debug, Clone)]
    struct PasteboardEntry {
        type_name: String,
        data: Vec<u8>,
    }

    pub fn is_accessibility_trusted() -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }

    pub fn capture_selected_text() -> Result<Option<String>> {
        match capture_accessibility() {
            AxOutcome::Selected(text) => Ok(Some(text)),
            // Accessibility 已明确告知当前焦点控件为空选区：不合成 Cmd+C，避免误触蜂鸣音与 320ms 轮询等待
            AxOutcome::EmptySelection => Ok(None),
            AxOutcome::Unavailable => capture_clipboard_selected_text(),
        }
    }

    fn capture_accessibility() -> AxOutcome {
        unsafe {
            if AXIsProcessTrusted() == 0 {
                return AxOutcome::Unavailable;
            }

            let system_element = AXUIElementCreateSystemWide();
            if system_element.is_null() {
                return AxOutcome::Unavailable;
            }

            let focused_attr = CFString::new("AXFocusedUIElement");
            let mut focused_value: CFTypeRef = ptr::null();
            let focused_error = AXUIElementCopyAttributeValue(
                system_element,
                focused_attr.as_concrete_TypeRef(),
                &mut focused_value,
            );
            CFRelease(system_element as CFTypeRef);

            if focused_error != AX_ERROR_SUCCESS || focused_value.is_null() {
                return AxOutcome::Unavailable;
            }

            let selected_attr = CFString::new("AXSelectedText");
            let mut selected_value: CFTypeRef = ptr::null();
            let selected_error = AXUIElementCopyAttributeValue(
                focused_value as AXUIElementRef,
                selected_attr.as_concrete_TypeRef(),
                &mut selected_value,
            );
            CFRelease(focused_value);

            if selected_error == AX_ERROR_NO_VALUE {
                return AxOutcome::EmptySelection;
            }
            if selected_error != AX_ERROR_SUCCESS || selected_value.is_null() {
                return AxOutcome::Unavailable;
            }

            let selected =
                CFString::wrap_under_create_rule(selected_value as CFStringRef).to_string();
            match normalize_selected_text(selected) {
                Some(text) => AxOutcome::Selected(text),
                None => AxOutcome::EmptySelection,
            }
        }
    }

    fn capture_clipboard_selected_text() -> Result<Option<String>> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let snapshot = snapshot_pasteboard(&pasteboard);
        let before_copy = pasteboard.changeCount();

        post_copy_keypress()?;

        let Some(copy_change_count) = wait_for_pasteboard_change(&pasteboard, before_copy) else {
            return Ok(None);
        };

        let copied_text = read_plain_text(&pasteboard).and_then(normalize_selected_text);

        if pasteboard.changeCount() == copy_change_count {
            restore_pasteboard(&pasteboard, &snapshot);
        }

        Ok(copied_text)
    }

    fn snapshot_pasteboard(pasteboard: &NSPasteboard) -> PasteboardSnapshot {
        let entries = pasteboard
            .types()
            .map(|types| {
                types
                    .to_vec()
                    .into_iter()
                    .filter_map(|type_name| {
                        pasteboard
                            .dataForType(&type_name)
                            .map(|data| PasteboardEntry {
                                type_name: type_name.to_string(),
                                data: data.to_vec(),
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();

        PasteboardSnapshot { entries }
    }

    fn restore_pasteboard(pasteboard: &NSPasteboard, snapshot: &PasteboardSnapshot) {
        let _ = pasteboard.clearContents();
        for entry in &snapshot.entries {
            let type_name = NSString::from_str(&entry.type_name);
            let data = NSData::with_bytes(&entry.data);
            let _ = pasteboard.setData_forType(Some(&data), &type_name);
        }
    }

    fn wait_for_pasteboard_change(
        pasteboard: &NSPasteboard,
        before_copy: NSInteger,
    ) -> Option<NSInteger> {
        let deadline = Instant::now() + COPY_TIMEOUT;
        while Instant::now() < deadline {
            let current = pasteboard.changeCount();
            if current != before_copy {
                return Some(current);
            }
            thread::sleep(COPY_POLL_INTERVAL);
        }
        None
    }

    fn read_plain_text(pasteboard: &NSPasteboard) -> Option<String> {
        let pb_type: &'static NSPasteboardType = unsafe { NSPasteboardTypeString };
        pasteboard
            .stringForType(pb_type)
            .map(|text| text.to_string())
    }

    fn post_copy_keypress() -> Result<()> {
        let key_down = CGEvent::new_keyboard_event(None, COPY_KEY_CODE, true)
            .ok_or_else(|| anyhow!("创建复制按下事件失败"))?;
        let key_up = CGEvent::new_keyboard_event(None, COPY_KEY_CODE, false)
            .ok_or_else(|| anyhow!("创建复制释放事件失败"))?;

        CGEvent::set_flags(Some(&key_down), CGEventFlags::MaskCommand);
        CGEvent::set_flags(Some(&key_up), CGEventFlags::MaskCommand);
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&key_down));
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&key_up));
        Ok(())
    }
}

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
mod win_imp {
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{anyhow, Result};

    use super::normalize_selected_text;

    const COPY_TIMEOUT: Duration = Duration::from_millis(280);
    const COPY_POLL_INTERVAL: Duration = Duration::from_millis(12);

    type HANDLE = *mut core::ffi::c_void;
    type BOOL = i32;
    const NULL_HANDLE: HANDLE = std::ptr::null_mut();

    const INPUT_KEYBOARD: u32 = 1;
    const KEYEVENTF_KEYUP: u32 = 2;
    const VK_SHIFT: u16 = 0x10;
    const VK_CONTROL: u16 = 0x11;
    const VK_MENU: u16 = 0x12;
    const VK_C: u16 = 0x43;
    const VK_LWIN: u16 = 0x5B;
    const VK_RWIN: u16 = 0x5C;
    const VK_LSHIFT: u16 = 0xA0;
    const VK_RSHIFT: u16 = 0xA1;
    const VK_LMENU: u16 = 0xA4;
    const VK_RMENU: u16 = 0xA5;
    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct KEYBDINPUT {
        wVk: u16,
        wScan: u16,
        dwFlags: u32,
        time: u32,
        dwExtraInfo: usize,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    union INPUT_0 {
        ki: KEYBDINPUT,
        _padding: [u8; 24],
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct INPUT {
        r#type: u32,
        Anonymous: INPUT_0,
    }

    extern "system" {
        fn OpenClipboard(hWndNewOwner: HANDLE) -> BOOL;
        fn CloseClipboard() -> BOOL;
        fn EmptyClipboard() -> BOOL;
        fn GetClipboardData(uFormat: u32) -> HANDLE;
        fn SetClipboardData(uFormat: u32, hMem: HANDLE) -> HANDLE;
        fn EnumClipboardFormats(format: u32) -> u32;
        fn GetClipboardSequenceNumber() -> u32;
        fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> HANDLE;
        fn GlobalLock(hMem: HANDLE) -> *mut core::ffi::c_void;
        fn GlobalUnlock(hMem: HANDLE) -> BOOL;
        fn GlobalSize(hMem: HANDLE) -> usize;
        fn GlobalFree(hMem: HANDLE) -> HANDLE;
        fn SendInput(cInputs: u32, pInputs: *const INPUT, cbSize: i32) -> u32;
    }

    struct ClipboardSnapshot {
        sequence: u32,
        unicode_bytes: Option<Vec<u8>>,
    }

    pub fn capture_selected_text() -> Result<Option<String>> {
        unsafe {
            let snapshot = snapshot_clipboard();
            post_copy_keypress()?;

            let Some(_new_seq) = wait_for_clipboard_change(snapshot.sequence) else {
                return Ok(None);
            };

            let copied_text = read_clipboard_text().and_then(normalize_selected_text);

            // 若剪贴板仅发生本次 Ctrl+C 一次更新，无感还原用户原剪贴板文本
            if OpenClipboard(NULL_HANDLE) != 0 {
                let current_seq = GetClipboardSequenceNumber();
                if current_seq == snapshot.sequence.wrapping_add(1) {
                    EmptyClipboard();
                    if let Some(ref data) = snapshot.unicode_bytes {
                        let hmem = GlobalAlloc(GMEM_MOVEABLE, data.len());
                        if !hmem.is_null() {
                            let ptr = GlobalLock(hmem);
                            if !ptr.is_null() {
                                std::ptr::copy_nonoverlapping(
                                    data.as_ptr(),
                                    ptr as *mut u8,
                                    data.len(),
                                );
                                GlobalUnlock(hmem);
                                SetClipboardData(CF_UNICODETEXT, hmem);
                            } else {
                                GlobalFree(hmem);
                            }
                        }
                    }
                }
                CloseClipboard();
            }

            Ok(copied_text)
        }
    }

    unsafe fn snapshot_clipboard() -> ClipboardSnapshot {
        let sequence = GetClipboardSequenceNumber();
        let mut unicode_bytes = None;

        if OpenClipboard(NULL_HANDLE) != 0 {
            let mut format = 0u32;
            loop {
                format = EnumClipboardFormats(format);
                if format == 0 {
                    break;
                }
                if format == CF_UNICODETEXT {
                    let handle = GetClipboardData(CF_UNICODETEXT);
                    if !handle.is_null() {
                        let size = GlobalSize(handle);
                        let ptr = GlobalLock(handle);
                        if !ptr.is_null() && size > 0 {
                            unicode_bytes =
                                Some(std::slice::from_raw_parts(ptr as *const u8, size).to_vec());
                            GlobalUnlock(handle);
                        }
                    }
                    break;
                }
            }
            CloseClipboard();
        }

        ClipboardSnapshot {
            sequence,
            unicode_bytes,
        }
    }

    unsafe fn read_clipboard_text() -> Option<String> {
        if OpenClipboard(NULL_HANDLE) == 0 {
            return None;
        }
        let mut text = None;
        let handle = GetClipboardData(CF_UNICODETEXT);
        if !handle.is_null() {
            let ptr = GlobalLock(handle);
            if !ptr.is_null() {
                let mut len = 0usize;
                let base = ptr as *const u16;
                let max_u16 = GlobalSize(handle) / 2;
                while len < max_u16 && *base.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(base, len);
                text = String::from_utf16(slice).ok();
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
        text
    }

    fn key_input(vk: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn post_copy_keypress() -> Result<()> {
        unsafe {
            // 先释放触发全局热键时用户可能仍按住的 Alt / Shift / Win 修饰键，防止合成出 Ctrl+Alt+C
            let inputs = [
                key_input(VK_MENU, KEYEVENTF_KEYUP),
                key_input(VK_LMENU, KEYEVENTF_KEYUP),
                key_input(VK_RMENU, KEYEVENTF_KEYUP),
                key_input(VK_SHIFT, KEYEVENTF_KEYUP),
                key_input(VK_LSHIFT, KEYEVENTF_KEYUP),
                key_input(VK_RSHIFT, KEYEVENTF_KEYUP),
                key_input(VK_LWIN, KEYEVENTF_KEYUP),
                key_input(VK_RWIN, KEYEVENTF_KEYUP),
                key_input(VK_CONTROL, 0),
                key_input(VK_C, 0),
                key_input(VK_C, KEYEVENTF_KEYUP),
                key_input(VK_CONTROL, KEYEVENTF_KEYUP),
            ];
            let sent = SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
            if sent != inputs.len() as u32 {
                return Err(anyhow!("SendInput 发送 Ctrl+C 不完整"));
            }
        }
        thread::sleep(Duration::from_millis(35));
        Ok(())
    }

    fn wait_for_clipboard_change(before: u32) -> Option<u32> {
        let deadline = Instant::now() + COPY_TIMEOUT;
        while Instant::now() < deadline {
            let current = unsafe { GetClipboardSequenceNumber() };
            if current != before {
                return Some(current);
            }
            thread::sleep(COPY_POLL_INTERVAL);
        }
        None
    }
}

#[cfg(target_os = "macos")]
pub fn capture_selected_text() -> Result<Option<String>> {
    imp::capture_selected_text()
}

#[cfg(target_os = "macos")]
pub fn is_accessibility_trusted() -> bool {
    imp::is_accessibility_trusted()
}

#[cfg(target_os = "windows")]
pub fn capture_selected_text() -> Result<Option<String>> {
    win_imp::capture_selected_text()
}

#[cfg(target_os = "windows")]
pub fn is_accessibility_trusted() -> bool {
    true
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn capture_selected_text() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn is_accessibility_trusted() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_whitespace_only_to_none() {
        assert_eq!(normalize_selected_text("   \n\t "), None);
        assert_eq!(
            normalize_selected_text("  hello world \n"),
            Some("hello world".to_string())
        );
    }
}
