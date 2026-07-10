//! 屏幕区域截图 —— 调用 macOS 系统 `screencapture -i` 让用户自由框选。

use std::path::Path;
use std::process::Command;
use tempfile::{NamedTempFile, TempPath};

fn screencapture_args() -> [&'static str; 3] {
    ["-i", "-x", "-r"]
}

/// 触发 Apple 自带的交互式区域截图，落到临时 PNG 文件。
///
/// 返回截图文件路径；用户按 Esc 取消选择时返回 `Ok(None)`。
pub fn capture_interactive() -> Result<Option<TempPath>, String> {
    let target = new_capture_target()?;

    let status = Command::new("/usr/sbin/screencapture")
        .args(screencapture_args())
        .arg(target.path())
        .status()
        .map_err(|e| format!("调用 screencapture 失败: {e}"))?;

    if !status.success() {
        return Err("截图进程异常退出".to_string());
    }

    // NamedTempFile 预先创建了空文件；Esc 取消时它仍为空。
    if !validate_capture_output(target.path())? {
        return Ok(None);
    }
    Ok(Some(target.into_temp_path()))
}

fn new_capture_target() -> Result<NamedTempFile, String> {
    tempfile::Builder::new()
        .prefix("ttree-shot-")
        .suffix(".png")
        .tempfile()
        .map_err(|e| format!("创建截图临时文件失败: {e}"))
}

/// 返回 false 表示用户取消（文件为空）；非普通文件一律拒绝。
fn validate_capture_output(path: &Path) -> Result<bool, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| format!("读取截图临时文件失败: {e}"))?;
    if !metadata.file_type().is_file() {
        return Err("截图输出不是普通文件".to_string());
    }
    Ok(metadata.len() > 0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn uses_native_interactive_screencapture_flags() {
        assert_eq!(screencapture_args(), ["-i", "-x", "-r"]);
    }

    #[test]
    fn capture_targets_are_random_private_regular_files() {
        let first = new_capture_target().unwrap();
        let second = new_capture_target().unwrap();

        assert_ne!(first.path(), second.path());
        assert!(first.path().is_file());
        assert!(second.path().is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = first.as_file().metadata().unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn empty_capture_file_is_treated_as_cancellation() {
        let target = new_capture_target().unwrap();

        assert!(!validate_capture_output(target.path()).unwrap());
    }

    #[test]
    fn non_regular_capture_output_is_rejected() {
        let target = new_capture_target().unwrap();
        let path = target.path().to_path_buf();
        drop(target);
        fs::create_dir(&path).unwrap();

        let result = validate_capture_output(&path);
        fs::remove_dir(&path).unwrap();
        assert!(result.is_err());
    }
}
