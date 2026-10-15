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
#[path = "macos/selection.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows/selection.rs"]
mod win_imp;

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
