#![allow(non_snake_case)]
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::selection::normalize_selected_text;

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
