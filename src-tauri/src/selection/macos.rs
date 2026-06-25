//! macOS 选中文本捕获实现。

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
const COPY_KEY_CODE: u16 = 8;
const COPY_TIMEOUT: Duration = Duration::from_millis(450);
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(20);

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

#[derive(Debug, Clone)]
struct PasteboardSnapshot {
    entries: Vec<PasteboardEntry>,
}

#[derive(Debug, Clone)]
struct PasteboardEntry {
    type_name: String,
    data: Vec<u8>,
}

pub(crate) fn capture_selected_text() -> Result<Option<String>> {
    if let Some(text) = capture_accessibility_selected_text()? {
        return Ok(Some(text));
    }

    capture_clipboard_selected_text()
}

fn capture_accessibility_selected_text() -> Result<Option<String>> {
    // SAFETY: We only call documented ApplicationServices Accessibility APIs. All returned
    // CoreFoundation objects that follow the Create/Copy rule are released or wrapped exactly once.
    unsafe {
        if AXIsProcessTrusted() == 0 {
            return Ok(None);
        }

        let system_element = AXUIElementCreateSystemWide();
        if system_element.is_null() {
            return Ok(None);
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
            return Ok(None);
        }

        let selected_attr = CFString::new("AXSelectedText");
        let mut selected_value: CFTypeRef = ptr::null();
        let selected_error = AXUIElementCopyAttributeValue(
            focused_value as AXUIElementRef,
            selected_attr.as_concrete_TypeRef(),
            &mut selected_value,
        );
        CFRelease(focused_value);

        if selected_error != AX_ERROR_SUCCESS || selected_value.is_null() {
            return Ok(None);
        }

        let selected = CFString::wrap_under_create_rule(selected_value as CFStringRef).to_string();
        Ok(normalize_selected_text(selected))
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

    if should_restore_pasteboard(pasteboard.changeCount(), copy_change_count) {
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
                    pasteboard.dataForType(&type_name).map(|data| PasteboardEntry {
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

fn should_restore_pasteboard(current_change_count: NSInteger, copy_change_count: NSInteger) -> bool {
    current_change_count == copy_change_count
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
    pasteboard
        .stringForType(string_pasteboard_type())
        .map(|text| text.to_string())
}

fn string_pasteboard_type() -> &'static NSPasteboardType {
    // SAFETY: NSPasteboardTypeString is an AppKit constant with static lifetime.
    unsafe { NSPasteboardTypeString }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasteboard_restore_requires_unchanged_copy_count() {
        assert!(should_restore_pasteboard(42, 42));
        assert!(!should_restore_pasteboard(43, 42));
    }
}
