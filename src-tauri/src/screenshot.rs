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

/// 截取全屏并保存到临时 BMP 文件（Windows）。
#[cfg(target_os = "windows")]
pub fn capture_fullscreen() -> Result<PathBuf, String> {
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    // SAFETY: Win32 GDI 对象在本函数作用域内有效，逐一释放。
    unsafe {
        let hdc_screen = GetDC(HWND::default());
        if hdc_screen.is_invalid() {
            return Err("获取屏幕 DC 失败".to_string());
        }

        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);

        let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
        if hdc_mem.is_invalid() {
            ReleaseDC(HWND::default(), hdc_screen);
            return Err("创建内存 DC 失败".to_string());
        }

        let hbitmap = CreateCompatibleBitmap(Some(hdc_screen), width, height);
        if hbitmap.is_invalid() {
            DeleteDC(hdc_mem);
            ReleaseDC(HWND::default(), hdc_screen);
            return Err("创建位图失败".to_string());
        }

        let old_bmp = SelectObject(hdc_mem, hbitmap.into());
        let _ = BitBlt(hdc_mem, 0, 0, width, height, Some(hdc_screen), 0, 0, SRCCOPY);

        // 保存为 BMP 文件
        let path = temp_path(".bmp");
        save_bitmap(&hbitmap, width, height, &path)?;

        // 清理
        let _ = SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(hbitmap.into());
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(HWND::default(), hdc_screen);

        Ok(path)
    }
}

/// Windows 交互式截图入口：截全屏，返回路径（后续由前端处理区域选择）。
#[cfg(target_os = "windows")]
pub fn capture_interactive() -> Result<Option<PathBuf>, String> {
    let path = capture_fullscreen()?;
    Ok(Some(path))
}

// ── GDI 工具函数 (Windows) ─────────────────────────────────────────

#[cfg(target_os = "windows")]
unsafe fn save_bitmap(
    hbitmap: &windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
    path: &PathBuf,
) -> Result<(), String> {
    use std::io::Write;
    use windows::Win32::Graphics::Gdi::*;

    // 获取位图信息
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB = 0
            ..std::mem::zeroed()
        },
        ..std::mem::zeroed()
    };

    let hdc = GetDC(HWND::default());
    let mut bits: Vec<u8> = vec![0; (width * height * 4) as usize];

    GetDIBits(
        Some(hdc),
        *hbitmap,
        0,
        height as u32,
        Some(bits.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );

    let _ = ReleaseDC(HWND::default(), hdc);

    // BMP 文件格式
    let file_size = 14 + 40 + bits.len();
    let mut file = std::fs::File::create(path).map_err(|e| format!("创建截图文件失败: {e}"))?;

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

    Ok(())
}
