//! Windows 选中文本捕获实现。
//!
//! 模拟 Ctrl+C + 剪贴板读取（与 macOS 的 Cmd+C 回退策略一致）。
//! 使用纯 Win32 FFI 避免 `windows` crate 版本间 API 差异。

#![cfg(target_os = "windows")]

use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::normalize_selected_text;

const COPY_TIMEOUT: Duration = Duration::from_millis(450);
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(20);

type HANDLE = *mut core::ffi::c_void;
type BOOL = i32;

const NULL_HANDLE: HANDLE = std::ptr::null_mut();
const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 2;
const VK_CONTROL: u16 = 0x11;
const VK_C: u16 = 0x43;
const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

#[repr(C)]
struct KEYBDINPUT {
    wVk: u16,
    wScan: u16,
    dwFlags: u32,
    time: u32,
    dwExtraInfo: usize,
}

#[repr(C)]
union INPUT_0 {
    ki: KEYBDINPUT,
    _padding: [u8; 24],
}

#[repr(C)]
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
        if OpenClipboard(NULL_HANDLE) != 0 {
            let current_seq = GetClipboardSequenceNumber();
            if current_seq == snapshot.sequence + 1 {
                // 只有一次变化（我们的 Ctrl+C），恢复原内容
                EmptyClipboard();
                for entry in &snapshot.entries {
                    if entry.format == CF_UNICODETEXT {
                        let hmem = GlobalAlloc(GMEM_MOVEABLE, entry.data.len());
                        if !hmem.is_null() {
                            let ptr = GlobalLock(hmem);
                            if !ptr.is_null() {
                                std::ptr::copy_nonoverlapping(
                                    entry.data.as_ptr(),
                                    ptr as *mut u8,
                                    entry.data.len(),
                                );
                                GlobalUnlock(hmem);
                                SetClipboardData(CF_UNICODETEXT, hmem);
                            } else {
                                GlobalFree(hmem);
                            }
                        }
                    }
                }
            }
            CloseClipboard();
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

    if OpenClipboard(NULL_HANDLE) != 0 {
        let mut format = 0u32;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                break;
            }
            let handle = GetClipboardData(format);
            if !handle.is_null() {
                let size = GlobalSize(handle);
                let ptr = GlobalLock(handle);
                if !ptr.is_null() && size > 0 {
                    let data = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
                    entries.push(ClipboardEntry { format, data });
                    GlobalUnlock(handle);
                }
            }
        }
        CloseClipboard();
    }

    ClipboardSnapshot { sequence, entries }
}

/// 读取剪贴板中的 Unicode 文本。
unsafe fn read_clipboard_text() -> Option<String> {
    if OpenClipboard(NULL_HANDLE) == 0 {
        return None;
    }

    let mut text = None;
    let handle = GetClipboardData(CF_UNICODETEXT);
    if !handle.is_null() {
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
            GlobalUnlock(handle);
        }
    }

    CloseClipboard();
    text
}

/// 模拟 Ctrl+C 按键。
fn post_copy_keypress() -> Result<()> {
    use anyhow::anyhow;

    // SAFETY: SendInput 使用合法的 INPUT 结构。
    unsafe {
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL,
                        wScan: 0,
                        dwFlags: 0,
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
                        dwFlags: 0,
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

        let sent = SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
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
