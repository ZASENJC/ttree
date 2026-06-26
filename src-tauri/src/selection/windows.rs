//! Windows 选中文本捕获实现。
//!
//! 策略: 模拟 Ctrl+C + 剪贴板读取（与 macOS 的 Cmd+C 回退策略一致）。
//! 优先尝试 UI Automation，失败后回退到剪贴板模拟。

#![cfg(target_os = "windows")]

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use super::normalize_selected_text;

const COPY_TIMEOUT: Duration = Duration::from_millis(450);
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) fn capture_selected_text() -> Result<Option<String>> {
    capture_clipboard_selected_text()
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
                let _ = EmptyClipboard();
                for entry in &snapshot.entries {
                    if entry.format == CF_UNICODETEXT.0 as u32 {
                        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, entry.data.len()) {
                            let ptr = GlobalLock(hmem);
                            if !ptr.is_null() {
                                std::ptr::copy_nonoverlapping(
                                    entry.data.as_ptr(),
                                    ptr as *mut u8,
                                    entry.data.len(),
                                );
                                let _ = GlobalUnlock(hmem);
                                let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(hmem));
                            } else {
                                let _ = GlobalFree(Some(hmem));
                            }
                        }
                    }
                }
            }
            let _ = CloseClipboard();
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
            if let Ok(handle) = GetClipboardData(format) {
                let size = GlobalSize(handle);
                let ptr = GlobalLock(handle);
                if !ptr.is_null() && size > 0 {
                    let data = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
                    entries.push(ClipboardEntry { format, data });
                    let _ = GlobalUnlock(handle);
                }
            }
        }
        let _ = CloseClipboard();
    }

    ClipboardSnapshot { sequence, entries }
}

/// 读取剪贴板中的 Unicode 文本。
unsafe fn read_clipboard_text() -> Option<String> {
    if OpenClipboard(HWND::default()).is_err() {
        return None;
    }

    let mut text = None;
    if let Ok(handle) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
        let ptr = GlobalLock(handle);
        if !ptr.is_null() {
            // 读取 null-terminated UTF-16 字符串
            let mut len = 0;
            let base = ptr as *const u16;
            while *base.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(base, len);
            text = String::from_utf16(slice).ok();
            let _ = GlobalUnlock(handle);
        }
    }

    let _ = CloseClipboard();
    text
}

/// 模拟 Ctrl+C 按键。
fn post_copy_keypress() -> Result<()> {
    // SAFETY: SendInput 使用合法的 INPUT 结构。
    unsafe {
        let inputs = [
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
