//! 系统选中文本捕获。
//!
//! macOS 上优先通过 Accessibility 读取当前焦点控件的选中文本，失败后回退到
//! 临时 Cmd+C + 剪贴板读取。
//!
//! Windows 上优先通过 UI Automation 读取，失败后回退到
//! 临时 Ctrl+C + 剪贴板读取。
//!
//! 其他平台返回空结果。

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos::capture_selected_text;

#[cfg(target_os = "windows")]
pub(crate) use windows::capture_selected_text;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn capture_selected_text() -> anyhow::Result<Option<String>> {
    Ok(None)
}

pub(crate) fn normalize_selected_text(text: impl AsRef<str>) -> Option<String> {
    let trimmed = text.as_ref().trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_blank_text_to_none() {
        assert_eq!(normalize_selected_text(" \n\t "), None);
    }

    #[test]
    fn trims_non_empty_selected_text() {
        assert_eq!(normalize_selected_text("  hello  "), Some("hello".to_string()));
    }
}
