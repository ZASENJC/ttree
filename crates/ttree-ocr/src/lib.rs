//! `ttree-ocr` —— macOS 原生截图与 Apple Vision OCR 引擎。
//!
//! 针对旧版“莫名其妙无法使用”的 4 项核心根治：
//! 1. 强类型 `OcrOutcome` 区分「成功」「用户取消」「未识别出文字」「录屏权限未开启」「正在截图中」，杜绝静默吞掉错误；
//! 2. 调用 `CGPreflightScreenCaptureAccess` 预检 macOS 录屏权限，区分无权限导致的空截图与按 Esc 取消；
//! 3. `image.rs` 自动采样截图边缘背景色填充 Padding（替代旧版硬编码纯白底，彻底修复深色模式单行小图识别失败）；
//! 4. `vision.rs` 移除主路径上的 `PreferBackgroundProcessing(true)` 后台降级标志，确保菜单栏配件应用极速响应。

pub mod screenshot;

#[cfg(target_os = "macos")]
pub mod image;
#[cfg(target_os = "macos")]
pub mod vision;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

static OCR_BUSY: AtomicBool = AtomicBool::new(false);

struct BusyGuard;

impl BusyGuard {
    fn try_acquire() -> Option<Self> {
        OCR_BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        OCR_BUSY.store(false, Ordering::SeqCst);
    }
}

/// 截图 OCR 强类型结果，彻底避免把权限失败或未识别出文字误判为用户取消。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcrOutcome {
    /// 成功识别出文字。
    Text(String),
    /// 用户按 Esc 主动取消了区域框选。
    Cancelled,
    /// 截图成功但所选图片区域内未检测到任何文字。
    EmptyText,
    /// 上一次截图框选仍在进行中。
    Busy,
    /// macOS 屏幕录制权限未授予或已失效。
    PermissionDenied(String),
}

/// 完整执行一次「macOS 原生交互式框选 (`screencapture -i -x -r`) + Apple Vision OCR」。
pub fn capture_and_recognize() -> Result<OcrOutcome, String> {
    let Some(_guard) = BusyGuard::try_acquire() else {
        return Ok(OcrOutcome::Busy);
    };

    match screenshot::capture_interactive()? {
        screenshot::CaptureResult::Captured(temp_path) => {
            recognize_file(&temp_path.to_string_lossy())
        }
        screenshot::CaptureResult::Cancelled => Ok(OcrOutcome::Cancelled),
        screenshot::CaptureResult::PermissionDenied(msg) => Ok(OcrOutcome::PermissionDenied(msg)),
    }
}

#[cfg(target_os = "macos")]
pub fn recognize_file(path: &str) -> Result<OcrOutcome, String> {
    vision::recognize_file(path)
}

#[cfg(not(target_os = "macos"))]
pub fn recognize_file(_path: &str) -> Result<OcrOutcome, String> {
    Err("当前平台暂不支持 OCR".to_string())
}
