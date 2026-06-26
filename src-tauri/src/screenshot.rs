//! 屏幕区域截图。
//!
//! - macOS: 调用系统 `screencapture -i` 让用户框选。
//! - Windows: 全屏截图到临时 BMP 文件，后续由前端处理区域选择。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// 全局自增序号，配合进程 id 生成唯一临时文件名，避免并发 OCR 互相覆盖。
static SHOT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成唯一的临时文件路径。
fn temp_path(suffix: &str) -> PathBuf {
    let seq = SHOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ttree_shot_{}_{}{suffix}",
        std::process::id(),
        seq
    ))
}

// ── macOS 实现 ──────────────────────────────────────────────────────

/// 触发交互式区域截图（macOS），落到临时 PNG 文件。
///
/// 返回截图文件路径；用户取消选择时返回 `Ok(None)`。
#[cfg(target_os = "macos")]
pub fn capture_interactive() -> Result<Option<PathBuf>, String> {
    use std::process::Command;

    let tmp = temp_path(".png");

    // -i 交互式框选，-x 静音，-r 不加窗口阴影
    let status = Command::new("/usr/sbin/screencapture")
        .args(["-i", "-x", "-r"])
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

// ── Windows 实现 ────────────────────────────────────────────────────

/// Windows 交互式截图入口：截全屏，返回路径（后续由前端处理区域选择）。
///
/// 使用纯 Win32 FFI 避免 `windows` crate 版本间 API 签名差异。
#[cfg(target_os = "windows")]
pub fn capture_interactive() -> Result<Option<PathBuf>, String> {
    use std::io::Write;

    // 纯 C FFI 类型
    type HANDLE = *mut core::ffi::c_void;
    type BOOL = i32;

    const NULL_HANDLE: HANDLE = std::ptr::null_mut();
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    const SRCCOPY: u32 = 0x00CC0020;
    const DIB_RGB_COLORS: u32 = 0;

    #[repr(C)]
    #[allow(dead_code)]
    struct BITMAPINFOHEADER {
        biSize: u32,
        biWidth: i32,
        biHeight: i32,
        biPlanes: u16,
        biBitCount: u16,
        biCompression: u32,
        biSizeImage: u32,
        biXPelsPerMeter: i32,
        biYPelsPerMeter: i32,
        biClrUsed: u32,
        biClrImportant: u32,
    }

    #[repr(C)]
    struct BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER,
    }

    extern "system" {
        fn GetDC(hWnd: HANDLE) -> HANDLE;
        fn ReleaseDC(hWnd: HANDLE, hDC: HANDLE) -> i32;
        fn CreateCompatibleDC(hDC: HANDLE) -> HANDLE;
        fn DeleteDC(hDC: HANDLE) -> BOOL;
        fn CreateCompatibleBitmap(hDC: HANDLE, cx: i32, cy: i32) -> HANDLE;
        fn SelectObject(hDC: HANDLE, h: HANDLE) -> HANDLE;
        fn DeleteObject(ho: HANDLE) -> BOOL;
        fn BitBlt(
            hdc: HANDLE, x: i32, y: i32, cx: i32, cy: i32,
            hdcSrc: HANDLE, x1: i32, y1: i32, rop: u32,
        ) -> BOOL;
        fn GetSystemMetrics(nIndex: i32) -> i32;
        fn GetDIBits(
            hdc: HANDLE, hbm: HANDLE, start: u32, cLines: u32,
            lpvBits: *mut core::ffi::c_void, lpbmi: *mut BITMAPINFO, usage: u32,
        ) -> i32;
    }

    // SAFETY: Win32 GDI 对象在本函数作用域内有效，逐一释放。
    unsafe {
        let hdc_screen = GetDC(NULL_HANDLE);
        if hdc_screen.is_null() {
            return Err("获取屏幕 DC 失败".to_string());
        }

        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);

        let hdc_mem = CreateCompatibleDC(hdc_screen);
        if hdc_mem.is_null() {
            ReleaseDC(NULL_HANDLE, hdc_screen);
            return Err("创建内存 DC 失败".to_string());
        }

        let hbitmap = CreateCompatibleBitmap(hdc_screen, width, height);
        if hbitmap.is_null() {
            DeleteDC(hdc_mem);
            ReleaseDC(NULL_HANDLE, hdc_screen);
            return Err("创建位图失败".to_string());
        }

        let old_bmp = SelectObject(hdc_mem, hbitmap);
        BitBlt(hdc_mem, 0, 0, width, height, hdc_screen, 0, 0, SRCCOPY);

        // 保存为 BMP 文件
        let path = temp_path(".bmp");

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
        };

        let mut bits: Vec<u8> = vec![0; (width * height * 4) as usize];

        GetDIBits(
            hdc_screen,
            hbitmap,
            0,
            height as u32,
            bits.as_mut_ptr() as *mut _,
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // BMP 文件格式
        let file_size = 14 + 40 + bits.len();
        let mut file =
            std::fs::File::create(&path).map_err(|e| format!("创建截图文件失败: {e}"))?;

        // BMP 文件头
        file.write_all(b"BM").map_err(|e| e.to_string())?;
        file.write_all(&(file_size as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&[0u8; 4]).map_err(|e| e.to_string())?;
        file.write_all(&54u32.to_le_bytes())
            .map_err(|e| e.to_string())?;

        // DIB 头
        file.write_all(&40u32.to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&width.to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&height.to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&1u16.to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&32u16.to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&0u32.to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&(bits.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&[0u8; 16]).map_err(|e| e.to_string())?;

        // BGRA 像素数据
        file.write_all(&bits).map_err(|e| e.to_string())?;

        // 清理
        SelectObject(hdc_mem, old_bmp);
        DeleteObject(hbitmap);
        DeleteDC(hdc_mem);
        ReleaseDC(NULL_HANDLE, hdc_screen);

        Ok(Some(path))
    }
}
