//! 屏幕区域截图 —— 调用 macOS 系统 `screencapture -i` 让用户框选。

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// 全局自增序号，配合进程 id 生成唯一临时文件名，避免并发 OCR 互相覆盖。
static SHOT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn screencapture_args() -> [&'static str; 3] {
    ["-i", "-x", "-r"]
}

/// 触发交互式区域截图，落到临时 PNG 文件。
///
/// 返回截图文件路径；用户取消选择时返回 `Ok(None)`。
pub fn capture_interactive() -> Result<Option<PathBuf>, String> {
    let seq = SHOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "ttree_shot_{}_{}.png",
        std::process::id(),
        seq
    ));

    // -i 交互式框选，-x 静音，-r 不加窗口阴影
    let status = Command::new("/usr/sbin/screencapture")
        .args(screencapture_args())
        .arg(&tmp)
        .status()
        .map_err(|e| format!("调用 screencapture 失败: {e}"))?;

    if !status.success() {
        return Err("截图进程异常退出".to_string());
    }

    // 用户按 Esc 取消时不会生成文件
    if !tmp.exists() {
        return Ok(None);
    }
    Ok(Some(tmp))
}

#[cfg(test)]
mod tests {
    use super::screencapture_args;

    #[test]
    fn uses_native_interactive_screencapture_flags() {
        assert_eq!(screencapture_args(), ["-i", "-x", "-r"]);
    }
}
