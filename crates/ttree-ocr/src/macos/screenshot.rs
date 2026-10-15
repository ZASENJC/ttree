//! macOS 原生屏幕区域框选（`/usr/sbin/screencapture -i -x -r`）与 TCC 录屏权限预检。

use std::path::Path;
use std::process::Command;
use tempfile::{NamedTempFile, TempPath};

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
pub fn has_screen_capture_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[cfg(target_os = "macos")]
pub fn request_screen_capture_permission() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

#[cfg(not(target_os = "macos"))]
pub fn has_screen_capture_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_screen_capture_permission() -> bool {
    true
}

pub fn screencapture_args() -> [&'static str; 3] {
    ["-i", "-x", "-r"]
}

pub enum CaptureResult {
    Captured(TempPath),
    Cancelled,
    PermissionDenied(String),
}

const PERMISSION_HINT: &str = "未获得 macOS 屏幕录制权限。请打开「系统设置 → 隐私与安全性 → 屏幕与系统音频录制」，开启 TTREE 权限后重试。";

/// 调用 Apple 原生 `/usr/sbin/screencapture -i -x -r` 进行自由框选截图。
pub fn capture_interactive() -> Result<CaptureResult, String> {
    let had_permission = has_screen_capture_permission();
    if !had_permission {
        // 主动请求一次系统权限弹窗，若仍未授权则稍后给出明确提示而非静默返回空
        let _ = request_screen_capture_permission();
    }

    let target = new_capture_target()?;

    let output = Command::new("/usr/sbin/screencapture")
        .args(screencapture_args())
        .arg(target.path())
        .output()
        .map_err(|e| format!("调用 screencapture 失败: {e}"))?;

    let has_capture = validate_capture_output(target.path())?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        if !has_capture
            && (detail.trim().is_empty() || detail.to_ascii_lowercase().contains("cancel"))
        {
            if !had_permission && !has_screen_capture_permission() {
                return Ok(CaptureResult::PermissionDenied(PERMISSION_HINT.to_string()));
            }
            return Ok(CaptureResult::Cancelled);
        }
        let detail: String = detail.trim().chars().take(300).collect();
        return Err(format!(
            "系统截图失败（{}）: {detail}。{PERMISSION_HINT}",
            output.status
        ));
    }

    if !has_capture {
        if !had_permission && !has_screen_capture_permission() {
            return Ok(CaptureResult::PermissionDenied(PERMISSION_HINT.to_string()));
        }
        return Ok(CaptureResult::Cancelled);
    }

    Ok(CaptureResult::Captured(target.into_temp_path()))
}

fn new_capture_target() -> Result<NamedTempFile, String> {
    tempfile::Builder::new()
        .prefix("ttree-shot-")
        .suffix(".png")
        .tempfile()
        .map_err(|e| format!("创建截图临时文件失败: {e}"))
}

fn validate_capture_output(path: &Path) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("读取截图临时文件失败: {error}")),
    };
    if !metadata.file_type().is_file() {
        return Err("截图输出不是普通文件".to_string());
    }
    Ok(metadata.len() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_native_screencapture_flags() {
        assert_eq!(screencapture_args(), ["-i", "-x", "-r"]);
    }

    #[test]
    fn empty_or_deleted_temp_file_is_not_captured() {
        let target = new_capture_target().unwrap();
        assert!(!validate_capture_output(target.path()).unwrap());
        let path = target.path().to_path_buf();
        drop(target);
        assert!(!validate_capture_output(&path).unwrap());
    }
}
