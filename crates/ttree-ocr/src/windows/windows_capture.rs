//! Windows 纯 Rust 原生全屏自由框选截图（零 Webview 依赖、Per-Monitor DPI v2 1:1 物理像素对齐）。
//!
//! 解决旧版 Windows 截图的三大痛点：
//! 1. **彻底消除 DPI 缩放偏移**：进入截图前将当前线程切换至 `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`，
//!    无论系统缩放为 100% / 125% / 150% / 200% 或多显示器异构缩放，GDI 截屏与鼠标框选坐标严格 1:1 对齐；
//! 2. **支持多显示器虚拟桌面**：使用 `SM_XVIRTUALSCREEN..SM_CXVIRTUALSCREEN` 覆盖全部显示器；
//! 3. **毫秒级顺滑框选**：预生成 45% 暗化背景缓冲，拖拽时直接内存合成高亮选区 + 2px 天蓝色边框，通过单次
//!    `SetDIBitsToDevice` 零闪烁输出，无需启动笨重的 Webview 窗口或落盘临时文件。

#![cfg(target_os = "windows")]
#![allow(non_snake_case)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;

use crate::preprocess::RawBgraImage;

pub enum WindowsCaptureResult {
    Captured(RawBgraImage),
    Cancelled,
}

type HWND = *mut c_void;
type HDC = *mut c_void;
type HBITMAP = *mut c_void;
type HGDIOBJ = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HINSTANCE = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;

const SRCCOPY: u32 = 0x00CC0020;
const CAPTUREBLT: u32 = 0x40000000;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;

const WS_POPUP: u32 = 0x80000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_EX_TOPMOST: u32 = 0x00000008;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;

const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_KEYDOWN: u32 = 0x0100;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;

const VK_ESCAPE: usize = 0x1B;
const IDC_CROSS: *const u16 = 32515 as *const u16;
const SW_SHOW: i32 = 5;

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

#[repr(C)]
#[derive(Copy, Clone)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: WPARAM,
    lParam: LPARAM,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    fErase: BOOL,
    rcPaint: [i32; 4],
    fRestore: BOOL,
    fIncUpdate: BOOL,
    rgbReserved: [u8; 32],
}

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: u32,
    style: u32,
    lpfnWndProc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: HICON,
    hCursor: HCURSOR,
    hbrBackground: HBRUSH,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: HICON,
}

#[repr(C)]
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
    bmiColors: [u32; 1],
}

extern "system" {
    fn SetThreadDpiAwarenessContext(dpiContext: isize) -> isize;
    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn GetDC(hWnd: HWND) -> HDC;
    fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HBITMAP;
    fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    fn DeleteObject(ho: HGDIOBJ) -> BOOL;
    fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: HDC,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> BOOL;
    fn GetDIBits(
        hdc: HDC,
        hbm: HBITMAP,
        start: u32,
        cLines: u32,
        lpvBits: *mut c_void,
        lpbmi: *mut BITMAPINFO,
        usage: u32,
    ) -> i32;
    fn SetDIBitsToDevice(
        hdc: HDC,
        xDest: i32,
        yDest: i32,
        w: u32,
        h: u32,
        xSrc: i32,
        ySrc: i32,
        StartScan: u32,
        cLines: u32,
        lpvBits: *const c_void,
        lpbmi: *const BITMAPINFO,
        ColorUse: u32,
    ) -> i32;

    fn GetModuleHandleW(lpModuleName: *const u16) -> HINSTANCE;
    fn RegisterClassExW(lpWndClass: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dwExStyle: u32,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: u32,
        X: i32,
        Y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: HWND,
        hMenu: *mut c_void,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hWnd: HWND) -> BOOL;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    fn SetForegroundWindow(hWnd: HWND) -> BOOL;
    fn SetFocus(hWnd: HWND) -> HWND;
    fn SetCapture(hWnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn InvalidateRect(hWnd: HWND, lpRect: *const i32, bErase: BOOL) -> BOOL;
    fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> BOOL;
    fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: u32, wMsgFilterMax: u32) -> BOOL;
    fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
    fn PostQuitMessage(nExitCode: i32);
}

struct OverlayState {
    width: u32,
    height: u32,
    orig_bgra: Vec<u8>,
    dimmed_bgra: Vec<u8>,
    frame_bgra: Vec<u8>,
    dragging: bool,
    start_x: i32,
    start_y: i32,
    curr_x: i32,
    curr_y: i32,
    selected_rect: Option<(u32, u32, u32, u32)>,
    cancelled: bool,
}

thread_local! {
    static OVERLAY_STATE: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[inline]
fn loword_i16(lparam: LPARAM) -> i32 {
    (lparam as u32 & 0xFFFF) as i16 as i32
}

#[inline]
fn hiword_i16(lparam: LPARAM) -> i32 {
    ((lparam as u32 >> 16) & 0xFFFF) as i16 as i32
}

fn normalized_rect(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    max_w: u32,
    max_h: u32,
) -> (u32, u32, u32, u32) {
    let left = x1.min(x2).clamp(0, max_w as i32) as u32;
    let right = x1.max(x2).clamp(0, max_w as i32) as u32;
    let top = y1.min(y2).clamp(0, max_h as i32) as u32;
    let bottom = y1.max(y2).clamp(0, max_h as i32) as u32;
    (
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
}

/// 执行一次原生全屏框选截图，返回内存中的 `RawBgraImage`。
pub fn capture_interactive_region() -> Result<WindowsCaptureResult, String> {
    unsafe {
        // 切换当前线程到 Per-Monitor DPI v2，确保截屏与鼠标坐标均为 1:1 物理像素
        let prev_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let result = run_capture_session();
        if prev_dpi != 0 {
            SetThreadDpiAwarenessContext(prev_dpi);
        }
        result
    }
}

unsafe fn run_capture_session() -> Result<WindowsCaptureResult, String> {
    let mut vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let mut vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let mut vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let mut vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if vw <= 0 || vh <= 0 {
        vx = 0;
        vy = 0;
        vw = GetSystemMetrics(SM_CXSCREEN);
        vh = GetSystemMetrics(SM_CYSCREEN);
    }
    if vw <= 0 || vh <= 0 {
        return Err("获取 Windows 屏幕分辨率失败".to_string());
    }

    let orig_bgra = capture_virtual_screen_bgra(vx, vy, vw, vh)?;
    let mut dimmed_bgra = orig_bgra.clone();
    for px in dimmed_bgra.chunks_exact_mut(4) {
        px[0] = ((px[0] as u16 * 115) >> 8) as u8;
        px[1] = ((px[1] as u16 * 115) >> 8) as u8;
        px[2] = ((px[2] as u16 * 115) >> 8) as u8;
        px[3] = 255;
    }
    let frame_bgra = dimmed_bgra.clone();

    OVERLAY_STATE.with(|cell| {
        *cell.borrow_mut() = Some(OverlayState {
            width: vw as u32,
            height: vh as u32,
            orig_bgra,
            dimmed_bgra,
            frame_bgra,
            dragging: false,
            start_x: 0,
            start_y: 0,
            curr_x: 0,
            curr_y: 0,
            selected_rect: None,
            cancelled: false,
        });
    });

    let hinstance = GetModuleHandleW(ptr::null());
    let class_name = wide_null("TTreeScreenCaptureOverlayClass");
    let cursor = LoadCursorW(ptr::null_mut(), IDC_CROSS);

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: overlay_wnd_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: ptr::null_mut(),
        hCursor: cursor,
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: ptr::null_mut(),
    };
    let _ = RegisterClassExW(&wc);

    let win_title = wide_null("TTREE Screenshot Selection");
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name.as_ptr(),
        win_title.as_ptr(),
        WS_POPUP | WS_VISIBLE,
        vx,
        vy,
        vw,
        vh,
        ptr::null_mut(),
        ptr::null_mut(),
        hinstance,
        ptr::null_mut(),
    );
    if hwnd.is_null() {
        OVERLAY_STATE.with(|cell| *cell.borrow_mut() = None);
        return Err("创建全屏截图框选窗口失败".to_string());
    }

    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(hwnd);

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    let state = OVERLAY_STATE
        .with(|cell| cell.borrow_mut().take())
        .ok_or_else(|| "截图框选状态丢失".to_string())?;

    if state.cancelled {
        return Ok(WindowsCaptureResult::Cancelled);
    }
    let Some((rx, ry, rw, rh)) = state.selected_rect else {
        return Ok(WindowsCaptureResult::Cancelled);
    };
    if rw < 4 || rh < 4 {
        return Ok(WindowsCaptureResult::Cancelled);
    }

    let full = RawBgraImage::new(state.width, state.height, state.orig_bgra)?;
    let cropped = full.crop(rx, ry, rw, rh)?;
    Ok(WindowsCaptureResult::Captured(cropped))
}

unsafe fn capture_virtual_screen_bgra(
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
) -> Result<Vec<u8>, String> {
    let hdc_screen = GetDC(ptr::null_mut());
    if hdc_screen.is_null() {
        return Err("获取屏幕 DC 失败".to_string());
    }

    let hdc_mem = CreateCompatibleDC(hdc_screen);
    if hdc_mem.is_null() {
        ReleaseDC(ptr::null_mut(), hdc_screen);
        return Err("创建内存 DC 失败".to_string());
    }

    let hbmp = CreateCompatibleBitmap(hdc_screen, vw, vh);
    if hbmp.is_null() {
        DeleteDC(hdc_mem);
        ReleaseDC(ptr::null_mut(), hdc_screen);
        return Err("创建屏幕位图失败".to_string());
    }

    let old_obj = SelectObject(hdc_mem, hbmp);
    let ok = BitBlt(
        hdc_mem,
        0,
        0,
        vw,
        vh,
        hdc_screen,
        vx,
        vy,
        SRCCOPY | CAPTUREBLT,
    );

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = vw;
    // 负值 biHeight 表示 Top-Down 位图，免去手动上下翻转
    bmi.bmiHeader.biHeight = -vh;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut bgra = vec![0u8; (vw as usize) * (vh as usize) * 4];
    let lines = if ok != 0 {
        GetDIBits(
            hdc_screen,
            hbmp,
            0,
            vh as u32,
            bgra.as_mut_ptr() as *mut c_void,
            &mut bmi,
            DIB_RGB_COLORS,
        )
    } else {
        0
    };

    SelectObject(hdc_mem, old_obj);
    DeleteObject(hbmp);
    DeleteDC(hdc_mem);
    ReleaseDC(ptr::null_mut(), hdc_screen);

    if ok == 0 || lines == 0 {
        return Err("抓取屏幕像素失败".to_string());
    }
    Ok(bgra)
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(ptr::null_mut(), IDC_CROSS));
            1
        }
        WM_KEYDOWN => {
            if wparam == VK_ESCAPE {
                OVERLAY_STATE.with(|cell| {
                    if let Some(st) = cell.borrow_mut().as_mut() {
                        st.cancelled = true;
                    }
                });
                ReleaseCapture();
                DestroyWindow(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_RBUTTONDOWN => {
            OVERLAY_STATE.with(|cell| {
                if let Some(st) = cell.borrow_mut().as_mut() {
                    st.cancelled = true;
                }
            });
            ReleaseCapture();
            DestroyWindow(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            let x = loword_i16(lparam);
            let y = hiword_i16(lparam);
            OVERLAY_STATE.with(|cell| {
                if let Some(st) = cell.borrow_mut().as_mut() {
                    st.dragging = true;
                    st.start_x = x;
                    st.start_y = y;
                    st.curr_x = x;
                    st.curr_y = y;
                }
            });
            SetCapture(hwnd);
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_MOUSEMOVE => {
            let x = loword_i16(lparam);
            let y = hiword_i16(lparam);
            let mut should_repaint = false;
            OVERLAY_STATE.with(|cell| {
                if let Some(st) = cell.borrow_mut().as_mut() {
                    if st.dragging {
                        st.curr_x = x;
                        st.curr_y = y;
                        should_repaint = true;
                    }
                }
            });
            if should_repaint {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let x = loword_i16(lparam);
            let y = hiword_i16(lparam);
            ReleaseCapture();
            let mut should_close = false;
            OVERLAY_STATE.with(|cell| {
                if let Some(st) = cell.borrow_mut().as_mut() {
                    if st.dragging {
                        st.dragging = false;
                        st.curr_x = x;
                        st.curr_y = y;
                        let rect = normalized_rect(
                            st.start_x, st.start_y, st.curr_x, st.curr_y, st.width, st.height,
                        );
                        if rect.2 >= 6 && rect.3 >= 6 {
                            st.selected_rect = Some(rect);
                            should_close = true;
                        }
                    }
                }
            });
            if should_close {
                DestroyWindow(hwnd);
            } else {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            OVERLAY_STATE.with(|cell| {
                if let Some(st) = cell.borrow_mut().as_mut() {
                    render_overlay_frame(hdc, st);
                }
            });
            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn render_overlay_frame(hdc: HDC, st: &mut OverlayState) {
    st.frame_bgra.copy_from_slice(&st.dimmed_bgra);

    if st.dragging {
        let (rx, ry, rw, rh) = normalized_rect(
            st.start_x, st.start_y, st.curr_x, st.curr_y, st.width, st.height,
        );
        if rw > 0 && rh > 0 {
            let stride = (st.width as usize) * 4;
            let row_bytes = (rw as usize) * 4;
            for row in ry..(ry + rh) {
                let offset = (row as usize) * stride + (rx as usize) * 4;
                st.frame_bgra[offset..offset + row_bytes]
                    .copy_from_slice(&st.orig_bgra[offset..offset + row_bytes]);
            }

            // 绘制 2px 天蓝色 (#38BDF8 -> B=0xF8, G=0xBD, R=0x38) 选区外框
            draw_rect_border_bgra(
                &mut st.frame_bgra,
                st.width,
                st.height,
                rx,
                ry,
                rw,
                rh,
                [0xF8, 0xBD, 0x38, 0xFF],
            );
        }
    }

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = st.width as i32;
    bmi.bmiHeader.biHeight = -(st.height as i32);
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    SetDIBitsToDevice(
        hdc,
        0,
        0,
        st.width,
        st.height,
        0,
        0,
        0,
        st.height,
        st.frame_bgra.as_ptr() as *const c_void,
        &bmi,
        DIB_RGB_COLORS,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_rect_border_bgra(
    buf: &mut [u8],
    img_w: u32,
    img_h: u32,
    rx: u32,
    ry: u32,
    rw: u32,
    rh: u32,
    color: [u8; 4],
) {
    let x_end = (rx + rw).min(img_w);
    let y_end = (ry + rh).min(img_h);
    let mut set_px = |x: u32, y: u32| {
        if x < img_w && y < img_h {
            let idx = ((y as usize) * (img_w as usize) + (x as usize)) * 4;
            buf[idx..idx + 4].copy_from_slice(&color);
        }
    };

    for t in 0..2u32 {
        let top_y = ry.saturating_add(t);
        let bot_y = y_end.saturating_sub(1 + t);
        for x in rx..x_end {
            set_px(x, top_y);
            set_px(x, bot_y);
        }
        let left_x = rx.saturating_add(t);
        let right_x = x_end.saturating_sub(1 + t);
        for y in ry..y_end {
            set_px(left_x, y);
            set_px(right_x, y);
        }
    }
}
