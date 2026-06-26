//! Windows 选中文本捕获实现。
//!
//! 两层策略:
//! 1. UI Automation (IUIAutomation): 读取焦点元素的选中文本
//! 2. 剪贴板模拟: SendInput 模拟 Ctrl+C → 读取剪贴板 → 恢复原内容

#![cfg(target_os = "windows")]

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::Ole::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use super::normalize_selected_text;

const COPY_TIMEOUT: Duration = Duration::from_millis(450);
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) fn capture_selected_text() -> Result<Option<String>> {
    // 策略 1: UI Automation 读取焦点元素选中文本
    if let Some(text) = capture_uia_selected_text()? {
        return Ok(Some(text));
    }

    // 策略 2: 模拟 Ctrl+C + 剪贴板
    capture_clipboard_selected_text()
}

/// 通过 UI Automation 读取当前焦点元素的选中文本。
fn capture_uia_selected_text() -> Result<Option<String>> {
    // SAFETY: COM 初始化和 UI Automation 对象在本函数作用域内有效。
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL)
                .map_err(|e| anyhow!("创建 UI Automation 实例失败: {e}"))?;

        let focused = match automation.GetFocusedElement() {
            Ok(el) => el,
            Err(_) => return Ok(None),
        };

        // 尝试 TextPattern 读取选中文本
        let pattern_id = UIA_TextPatternId;
        if let Ok(pattern_variant) = focused.GetCurrentPattern(pattern_id) {
            if let Ok(text_pattern) = pattern_variant.cast::<IUIAutomationTextPattern>() {
                if let Ok(text_range) = text_pattern.GetSelection() {
                    if let Ok(count) = text_range.Length() {
                        if count > 0 {
                            if let Ok(text) = text_range.GetText(count.min(4096)) {
                                return Ok(normalize_selected_text(text.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // 回退: ValuePattern
        let value_pattern_id = UIA_ValuePatternId;
        if let Ok(pattern_variant) = focused.GetCurrentPattern(value_pattern_id) {
            if let Ok(value_pattern) = pattern_variant.cast::<IUIAutomationValuePattern>() {
                if let Ok(value) = value_pattern.CurrentValue() {
                    return Ok(normalize_selected_text(value.to_string()));
                }
            }
        }

        Ok(None)
    }
}

/// 模拟 Ctrl+C 并从剪贴板读取选中文本。
fn capture_clipboard_selected_text() -> Result<Option<String>> {
    // SAFETY: Win32 剪贴板和 SendInput API 在本函数作用域内使用。
    unsafe {
        // 保存当前剪贴板内容
        let snapshot = snapshot_clipboard();

        // 模拟 Ctrl+C
        post_copy_keypress()?;

        // 等待剪贴板变化
        let Some(_new_seq) = wait_for_clipboard_change(snapshot.sequence) else {
            return Ok(None);
        };

        // 读取剪贴板文本
        let copied_text = read_clipboard_text().and_then(normalize_selected_text);

        // 如果剪贴板没有被其他进程修改，恢复原始内容
        if OpenClipboard(HWND::default()).is_ok() {
            let current_seq = GetClipboardSequenceNumber();
            if current_seq == snapshot.sequence + 1 {
                // 只有一次变化（我们的 Ctrl+C），恢复原内容
                EmptyClipboard().ok();
                for entry in &snapshot.entries {
                    if entry.format == CF_UNICODETEXT {
                        let hmem = GlobalAlloc(GMEM_MOVEABLE, entry.data.len())
                            .ok()
                            .and_then(|h| {
                                let ptr = GlobalLock(h);
                                if !ptr.is_null() {
                                    std::ptr::copy_nonoverlapping(
                                        entry.data.as_ptr(),
                                        ptr as *mut u8,
                                        entry.data.len(),
                                    );
                                    GlobalUnlock(h).ok();
                                    Some(h)
                                } else {
                                    GlobalFree(Some(h));
                                    None
                                }
                            });
                        if let Some(h) = hmem {
                            SetClipboardData(entry.format, Some(h)).ok();
                        }
                    }
                }
            }
            CloseClipboard().ok();
        }

        Ok(copied_text)
    }
}

struct ClipboardSnapshot {
    sequence: u32,
    entries: Vec<ClipboardEntry>,
}

struct ClipboardEntry {
    format: u32,
    data: Vec<u8>,
}

/// 快照当前剪贴板内容。
unsafe fn snapshot_clipboard() -> ClipboardSnapshot {
    let sequence = GetClipboardSequenceNumber();
    let mut entries = Vec::new();

    if OpenClipboard(HWND::default()).is_ok() {
        let mut format = 0u32;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                break;
            }
            if let Some(handle) = GetClipboardData(format) {
                let hmem = handle.0 as isize;
                let size = GlobalSize(HGLOBAL(hmem));
                let ptr = GlobalLock(HGLOBAL(hmem));
                if !ptr.is_null() && size > 0 {
                    let data =
                        std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
                    entries.push(ClipboardEntry { format, data });
                    GlobalUnlock(HGLOBAL(hmem)).ok();
                }
            }
        }
        CloseClipboard().ok();
    }

    ClipboardSnapshot { sequence, entries }
}

/// 读取剪贴板中的 Unicode 文本。
unsafe fn read_clipboard_text() -> Option<String> {
    if OpenClipboard(HWND::default()).is_err() {
        return None;
    }

    let mut text = None;
    if let Some(handle) = GetClipboardData(CF_UNICODETEXT) {
        let hmem = handle.0 as isize;
        let ptr = GlobalLock(HGLOBAL(hmem));
        if !ptr.is_null() {
            // 读取 null-terminated UTF-16 字符串
            let mut len = 0;
            let base = ptr as *const u16;
            while *base.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(base, len);
            text = String::from_utf16(slice).ok();
            GlobalUnlock(HGLOBAL(hmem)).ok();
        }
    }

    CloseClipboard().ok();
    text
}

/// 模拟 Ctrl+C 按键。
fn post_copy_keypress() -> Result<()> {
    // SAFETY: SendInput 使用合法的 INPUT 结构。
    unsafe {
        let mut inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL,
                        wScan: 0,
                        dwFlags: KEYBD_EVENT_FLAGS(0),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_C,
                        wScan: 0,
                        dwFlags: KEYBD_EVENT_FLAGS(0),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_C,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];

        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent != inputs.len() as u32 {
            return Err(anyhow!("SendInput 发送不完整"));
        }
    }

    // 等待一小段时间让目标应用处理按键
    thread::sleep(Duration::from_millis(50));
    Ok(())
}

/// 等待剪贴板序号变化。
fn wait_for_clipboard_change(before: u32) -> Option<u32> {
    let deadline = Instant::now() + COPY_TIMEOUT;
    while Instant::now() < deadline {
        // SAFETY: GetClipboardSequenceNumber 是无害的查询 API。
        let current = unsafe { GetClipboardSequenceNumber() };
        if current != before {
            return Some(current);
        }
        thread::sleep(COPY_POLL_INTERVAL);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_entry_fields_work() {
        let entry = ClipboardEntry {
            format: 13, // CF_UNICODETEXT
            data: vec![0x48, 0x00, 0x69, 0x00], // "Hi" in UTF-16LE
        };
        assert_eq!(entry.format, 13);
        assert_eq!(entry.data.len(), 4);
    }
}
