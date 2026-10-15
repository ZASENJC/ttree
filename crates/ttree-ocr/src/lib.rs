//! `ttree-ocr` —— macOS 原生截图与 Apple Vision OCR 引擎。
//!
//! 针对旧版“莫名其妙无法使用”的 4 项核心根治：
//! 1. 强类型 `OcrOutcome` 区分「成功」「用户取消」「未识别出文字」「录屏权限未开启」「正在截图中」，杜绝静默吞掉错误；
//! 2. 调用 `CGPreflightScreenCaptureAccess` 预检 macOS 录屏权限，区分无权限导致的空截图与按 Esc 取消；
//! 3. `image.rs` 自动采样截图边缘背景色填充 Padding（替代旧版硬编码纯白底，彻底修复深色模式单行小图识别失败）；
//! 4. `vision.rs` 移除主路径上的 `PreferBackgroundProcessing(true)` 后台降级标志，确保菜单栏配件应用极速响应。

pub mod postprocess;
pub mod preprocess;
#[cfg(any(target_os = "macos", test))]
#[path = "macos/screenshot.rs"]
pub mod screenshot;

#[cfg(target_os = "macos")]
#[path = "macos/image.rs"]
pub mod image;
#[cfg(target_os = "macos")]
#[path = "macos/vision.rs"]
pub mod vision;

#[cfg(target_os = "windows")]
#[path = "windows/windows_capture.rs"]
pub mod windows_capture;
#[cfg(target_os = "windows")]
#[path = "windows/windows_ocr.rs"]
pub mod windows_ocr;

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
    /// 屏幕录制权限未授予，或系统缺少 OCR 语言包。
    PermissionDenied(String),
}

/// 完整执行一次「原生交互式自由框选 + 原生离线 OCR 识别」。
/// - macOS：调用 `/usr/sbin/screencapture -i -x -r` + Apple Vision OCR
/// - Windows：调用 Per-Monitor DPI v2 原生全屏框选 + 双通道增强多语言 `Windows.Media.Ocr`
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "windows")]
pub fn capture_and_recognize() -> Result<OcrOutcome, String> {
    let Some(_guard) = BusyGuard::try_acquire() else {
        return Ok(OcrOutcome::Busy);
    };

    match windows_capture::capture_interactive_region()? {
        windows_capture::WindowsCaptureResult::Captured(raw_img) => {
            windows_ocr::recognize_raw_bgra(raw_img)
        }
        windows_capture::WindowsCaptureResult::Cancelled => Ok(OcrOutcome::Cancelled),
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn capture_and_recognize() -> Result<OcrOutcome, String> {
    Err("当前平台暂不支持截图 OCR".to_string())
}

#[cfg(target_os = "macos")]
pub fn recognize_file(path: &str) -> Result<OcrOutcome, String> {
    vision::recognize_file(path)
}

#[cfg(target_os = "windows")]
pub fn recognize_file(path: &str) -> Result<OcrOutcome, String> {
    windows_ocr::recognize_file(path)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn recognize_file(_path: &str) -> Result<OcrOutcome, String> {
    Err("当前平台暂不支持 OCR".to_string())
}
