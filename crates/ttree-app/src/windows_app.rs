//! `ttree-app` Windows 100% 纯 Rust 原生 Win32 + DWM 桌面端实现。
//!
//! 架构与 macOS AppKit 端完全对称：
//! - 复用 `ttree-core`、`ttree-storage`、`ttree-translate`、`ttree-ai`、`ttree-ocr`、`ttree-platform-macos`；
//! - 原生 Win32 控件树 + DWM 深色模式 & Mica/Acrylic 云母材质 + 分层窗口透明度 (`SetLayeredWindowAttributes`)；
//! - 系统托盘 (`Shell_NotifyIconW`) + 25ms `SetTimer` 驱动 `global-hotkey` 与 Tokio `mpsc` 流式 UI 事件；
//! - 完整支持四段式翻译面板（含可拖拽高度分隔条）、AI 多轮流式对话、内置 `HY-MT1.5-1.8B` 一键下载托管、
//!   以及增强版双通道多语言 Windows 截图 OCR。

#![cfg(target_os = "windows")]
#![allow(non_snake_case, clippy::too_many_arguments)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use ttree_ai::hy_mt_manager::{self, HyMtCompatibilityReport, HyMtDownloadProgress};
use ttree_core::{
    normalize_ocr_newlines, recent_chat_context, resolve_language_pair, AiBackendKind,
    AiTranslationConfig, AppearanceConfig, CancelSlot, ChatAiConfig, ChatMessage, Conversation,
    HyMtQuant, PaneLayoutConfig, ShortcutConfig, ADAPTIVE_TARGET, PANEL_OPACITY_MAX,
    PANEL_OPACITY_MIN, SOURCE_LANGUAGES, TARGET_LANGUAGES, TOP_BAR_HEIGHT, WINDOW_MAX_HEIGHT,
    WINDOW_MAX_WIDTH, WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
use ttree_ocr::OcrOutcome;
use ttree_platform_macos::autostart;
use ttree_platform_macos::hotkey::{parse_shortcut_config, ParsedShortcuts, ShortcutAction};
use ttree_platform_macos::selection::capture_selected_text;
use ttree_storage::{HistoryStore, SettingsStore};

type HWND = *mut c_void;
type HMENU = *mut c_void;
type HINSTANCE = *mut c_void;
type HDC = *mut c_void;
type HFONT = *mut c_void;
type HBRUSH = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HANDLE = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;

const WS_OVERLAPPED: u32 = 0x00000000;
const WS_CAPTION: u32 = 0x00C00000;
const WS_SYSMENU: u32 = 0x00080000;
const WS_THICKFRAME: u32 = 0x00040000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_CHILD: u32 = 0x40000000;
const WS_VSCROLL: u32 = 0x00200000;
const WS_BORDER: u32 = 0x00800000;

const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_LAYERED: u32 = 0x00080000;

const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_READONLY: u32 = 0x0800;
const ES_PASSWORD: u32 = 0x0020;
const ES_WANTRETURN: u32 = 0x1000;

const BS_PUSHBUTTON: u32 = 0x0000;
const BS_AUTOCHECKBOX: u32 = 0x0003;

const CBS_DROPDOWNLIST: u32 = 0x0003;
const CBS_HASSTRINGS: u32 = 0x0200;

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_ACTIVATE: u32 = 0x0006;
const WM_CLOSE: u32 = 0x0010;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_SETFONT: u32 = 0x0030;
const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const WM_CTLCOLORLISTBOX: u32 = 0x0134;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_APP: u32 = 0x8000;
const WM_TRAYICON: u32 = WM_APP + 1;

const WA_INACTIVE: usize = 0;
const EN_CHANGE: u16 = 0x0300;
const CBN_SELCHANGE: u16 = 1;
const BN_CLICKED: u16 = 0;

const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CB_RESETCONTENT: u32 = 0x014B;

const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;
const BST_UNCHECKED: usize = 0;

const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const SW_RESTORE: i32 = 9;

const HWND_TOPMOST: HWND = -1isize as HWND;
const HWND_NOTOPMOST: HWND = -2isize as HWND;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOACTIVATE: u32 = 0x0010;

const LWA_ALPHA: u32 = 0x00000002;
const TRANSPARENT: i32 = 1;
const OPAQUE: i32 = 2;

const NIM_ADD: u32 = 0x00000000;
const NIM_DELETE: u32 = 0x00000002;
const NIF_MESSAGE: u32 = 0x00000001;
const NIF_ICON: u32 = 0x00000002;
const NIF_TIP: u32 = 0x00000004;

const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_BOTTOMALIGN: u32 = 0x0020;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;

const IDI_APPLICATION: *const u16 = 32512 as *const u16;
const IDC_ARROW: *const u16 = 32512 as *const u16;
const IDC_SIZENS: *const u16 = 32645 as *const u16;

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMWCP_ROUND: u32 = 2;
const DWMSBT_MAINWINDOW_MICA: u32 = 2;

const TIMER_ID_POLL: usize = 1;
const AUTO_TRANSLATE_DEBOUNCE: Duration = Duration::from_millis(650);
const RESIZE_SAVE_DEBOUNCE: Duration = Duration::from_millis(350);

// 控件 ID 定义
const ID_BTN_TAB_TRANS: usize = 101;
const ID_BTN_TAB_CHAT: usize = 102;
const ID_BTN_TAB_SETTINGS: usize = 103;
const ID_BTN_OCR: usize = 104;
const ID_BTN_PIN: usize = 105;

const ID_EDIT_SOURCE: usize = 201;
const ID_CMB_SOURCE_LANG: usize = 202;
const ID_BTN_SWAP_LANG: usize = 203;
const ID_CMB_TARGET_LANG: usize = 204;
const ID_BTN_TRANS_NOW: usize = 205;
const ID_BTN_CLEAR_TRANS: usize = 206;
const ID_BTN_COPY_WEB: usize = 207;
const ID_EDIT_WEB_RESULT: usize = 208;
const ID_BTN_COPY_AI: usize = 209;
const ID_EDIT_AI_RESULT: usize = 210;

const ID_CMB_CHAT_CONV: usize = 301;
const ID_BTN_CHAT_NEW: usize = 302;
const ID_BTN_CHAT_COPY: usize = 303;
const ID_BTN_CHAT_CLEAR: usize = 304;
const ID_EDIT_CHAT_HISTORY: usize = 305;
const ID_EDIT_CHAT_INPUT: usize = 306;
const ID_BTN_CHAT_SEND: usize = 307;
const ID_BTN_CHAT_STOP: usize = 308;

const ID_CHK_AI_ENABLED: usize = 401;
const ID_CMB_AI_BACKEND: usize = 402;
const ID_CMB_HYMT_QUANT: usize = 403;
const ID_BTN_HYMT_CHECK: usize = 404;
const ID_BTN_HYMT_RUN: usize = 405;
const ID_BTN_HYMT_STOP: usize = 406;
const ID_EDIT_AI_URL: usize = 407;
const ID_EDIT_AI_KEY: usize = 408;
const ID_EDIT_AI_MODEL: usize = 409;
const ID_BTN_AI_DISCOVER: usize = 410;
const ID_EDIT_AI_PROMPT: usize = 411;
const ID_CMB_CHAT_BACKEND: usize = 412;
const ID_EDIT_CHAT_URL: usize = 413;
const ID_EDIT_CHAT_KEY: usize = 414;
const ID_EDIT_CHAT_MODEL: usize = 415;
const ID_BTN_CHAT_COPY_TRANS: usize = 416;
const ID_EDIT_HK_TOGGLE: usize = 417;
const ID_EDIT_HK_OCR: usize = 418;
const ID_EDIT_HK_CHAT: usize = 419;
const ID_EDIT_HK_SEL_TRANS: usize = 420;
const ID_EDIT_HK_SEL_CHAT: usize = 421;
const ID_BTN_HK_RESET: usize = 422;
const ID_CMB_OPACITY: usize = 423;
const ID_CHK_OCR_NEWLINE: usize = 424;
const ID_CHK_AUTOSTART: usize = 425;
const ID_BTN_SAVE_SETTINGS: usize = 426;

const ID_TRAY_TRANSLATE: usize = 901;
const ID_TRAY_OCR: usize = 902;
const ID_TRAY_CHAT: usize = 903;
const ID_TRAY_SETTINGS: usize = 904;
const ID_TRAY_QUIT: usize = 905;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabMode {
    Translate,
    Chat,
    Settings,
}

enum UiEvent {
    WebTranslateDone {
        seq: u64,
        result: Result<String, String>,
    },
    AiTranslateDelta {
        seq: u64,
        delta: String,
    },
    AiTranslateDone {
        seq: u64,
        result: Result<(), String>,
    },
    ChatDelta {
        seq: u64,
        delta: String,
    },
    ChatDone {
        seq: u64,
        result: Result<(), String>,
    },
    OcrDone(Result<OcrOutcome, String>),
    ModelsDiscovered(Result<Vec<String>, String>),
    HyMtProgress(HyMtDownloadProgress),
    HyMtReady(Result<String, String>),
    HyMtCompatReady(HyMtCompatibilityReport),
}

#[repr(C)]
#[derive(Copy, Clone)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct MINMAXINFO {
    ptReserved: POINT,
    ptMaxSize: POINT,
    ptMaxPosition: POINT,
    ptMinTrackSize: POINT,
    ptMaxTrackSize: POINT,
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
struct NOTIFYICONDATAW {
    cbSize: u32,
    hWnd: HWND,
    uID: u32,
    uFlags: u32,
    uCallbackMessage: u32,
    hIcon: HICON,
    szTip: [u16; 128],
    dwState: u32,
    dwStateMask: u32,
    szInfo: [u16; 256],
    uTimeoutOrVersion: u32,
    szInfoTitle: [u16; 64],
    dwInfoFlags: u32,
    guidItem: [u8; 16],
    hBalloonIcon: HICON,
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: HWND,
        dwAttribute: u32,
        pvAttribute: *const c_void,
        cbAttribute: u32,
    ) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(dwMessage: u32, lpData: *const NOTIFYICONDATAW) -> BOOL;
}

extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> BOOL;
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
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hWnd: HWND) -> BOOL;
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    fn IsWindowVisible(hWnd: HWND) -> BOOL;
    fn SetForegroundWindow(hWnd: HWND) -> BOOL;
    fn SetFocus(hWnd: HWND) -> HWND;
    fn SetWindowPos(
        hWnd: HWND,
        hWndInsertAfter: HWND,
        X: i32,
        Y: i32,
        cx: i32,
        cy: i32,
        uFlags: u32,
    ) -> BOOL;
    fn MoveWindow(hWnd: HWND, X: i32, Y: i32, nWidth: i32, nHeight: i32, bRepaint: BOOL) -> BOOL;
    fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    fn SetLayeredWindowAttributes(hWnd: HWND, crKey: u32, bAlpha: u8, dwFlags: u32) -> BOOL;
    fn SetTimer(hWnd: HWND, nIDEvent: usize, uElapse: u32, lpTimerFunc: *const c_void) -> usize;
    fn SendMessageW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn SetWindowTextW(hWnd: HWND, lpString: *const u16) -> BOOL;
    fn GetWindowTextLengthW(hWnd: HWND) -> i32;
    fn GetWindowTextW(hWnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
    fn DefWindowProcW(hWnd: HWND, Msg: u32, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: u32, wMsgFilterMax: u32) -> BOOL;
    fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
    fn PostQuitMessage(nExitCode: i32);
    fn LoadIconW(hInstance: HINSTANCE, lpIconName: *const u16) -> HICON;
    fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
    fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
    fn SetCapture(hWnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: BOOL) -> BOOL;
    fn GetCursorPos(lpPoint: *mut POINT) -> BOOL;
    fn CreatePopupMenu() -> HMENU;
    fn AppendMenuW(hMenu: HMENU, uFlags: u32, uIDNewItem: usize, lpNewItem: *const u16) -> BOOL;
    fn TrackPopupMenu(
        hMenu: HMENU,
        uFlags: u32,
        x: i32,
        y: i32,
        nReserved: i32,
        hWnd: HWND,
        prcRect: *const RECT,
    ) -> BOOL;
    fn DestroyMenu(hMenu: HMENU) -> BOOL;

    fn CreateSolidBrush(color: u32) -> HBRUSH;
    fn CreateFontW(
        cHeight: i32,
        cWidth: i32,
        cEscapement: i32,
        cOrientation: i32,
        cWeight: i32,
        bItalic: u32,
        bUnderline: u32,
        bStrikeOut: u32,
        iCharSet: u32,
        iOutPrecision: u32,
        iClipPrecision: u32,
        iQuality: u32,
        iPitchAndFamily: u32,
        pszFaceName: *const u16,
    ) -> HFONT;
    fn SetTextColor(hdc: HDC, color: u32) -> u32;
    fn SetBkColor(hdc: HDC, color: u32) -> u32;
    fn SetBkMode(hdc: HDC, mode: i32) -> i32;

    fn OpenClipboard(hWndNewOwner: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn SetClipboardData(uFormat: u32, hMem: HANDLE) -> HANDLE;
    fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> HANDLE;
    fn GlobalLock(hMem: HANDLE) -> *mut c_void;
    fn GlobalUnlock(hMem: HANDLE) -> BOOL;
    fn GlobalFree(hMem: HANDLE) -> HANDLE;
}

#[inline]
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn to_crlf(s: &str) -> String {
    let normalized = s.replace("\r\n", "\n");
    normalized.replace('\n', "\r\n")
}

fn from_crlf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

unsafe fn set_hwnd_text(hwnd: HWND, text: &str) {
    if !hwnd.is_null() {
        let w = wide_null(&to_crlf(text));
        SetWindowTextW(hwnd, w.as_ptr());
    }
}

unsafe fn get_hwnd_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 2];
    let read = GetWindowTextW(hwnd, buf.as_mut_ptr(), len + 1);
    if read <= 0 {
        return String::new();
    }
    from_crlf(&String::from_utf16_lossy(&buf[..read as usize]))
}

unsafe fn copy_text_to_clipboard(hwnd: HWND, text: &str) {
    if text.is_empty() {
        return;
    }
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = utf16.len() * 2;
    if OpenClipboard(hwnd) != 0 {
        EmptyClipboard();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if !hmem.is_null() {
            let ptr = GlobalLock(hmem);
            if !ptr.is_null() {
                ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, ptr as *mut u8, byte_len);
                GlobalUnlock(hmem);
                SetClipboardData(CF_UNICODETEXT, hmem);
            } else {
                GlobalFree(hmem);
            }
        }
        CloseClipboard();
    }
}

struct Win32AppState {
    rt: TokioHandle,
    ui_tx: UnboundedSender<UiEvent>,
    ui_rx: UnboundedReceiver<UiEvent>,
    settings: Arc<SettingsStore>,
    history: Arc<HistoryStore>,
    hotkey_mgr: GlobalHotKeyManager,
    registered_hotkeys: Vec<HotKey>,
    parsed_shortcuts: ParsedShortcuts,

    hwnd: HWND,
    font_regular: HFONT,
    font_bold: HFONT,
    brush_bg: HBRUSH,
    brush_surface: HBRUSH,

    active_tab: TabMode,
    pinned: bool,
    pin_before_settings: bool,
    ocr_in_flight: bool,
    suppress_source_change: bool,

    source_lang_idx: usize,
    target_lang_idx: usize,
    pane_layout: PaneLayoutConfig,
    dragging_splitter: Option<(u8, i32)>,

    translate_seq: u64,
    web_cancel: CancelSlot,
    ai_cancel: CancelSlot,
    ai_stream_buf: String,
    web_result_text: String,

    chat_seq: u64,
    chat_cancel: CancelSlot,
    chat_streaming: bool,
    conversations: Vec<Conversation>,
    active_conv_idx: usize,
    current_messages: Vec<ChatMessage>,
    streaming_reply: String,

    hymt_cancel: CancelSlot,

    pending_auto_translate: Option<Instant>,
    pending_resize_save: Option<(Instant, f64, f64)>,

    // Top bar controls
    btn_tab_trans: HWND,
    btn_tab_chat: HWND,
    btn_tab_settings: HWND,
    btn_ocr: HWND,
    btn_pin: HWND,

    // Translate controls
    edit_source: HWND,
    cmb_source_lang: HWND,
    btn_swap_lang: HWND,
    cmb_target_lang: HWND,
    btn_trans_now: HWND,
    btn_clear_trans: HWND,
    lbl_web_header: HWND,
    btn_copy_web: HWND,
    edit_web_result: HWND,
    lbl_ai_header: HWND,
    btn_copy_ai: HWND,
    edit_ai_result: HWND,

    // Chat controls
    cmb_chat_conv: HWND,
    btn_chat_new: HWND,
    btn_chat_copy: HWND,
    btn_chat_clear: HWND,
    edit_chat_history: HWND,
    edit_chat_input: HWND,
    btn_chat_send: HWND,
    btn_chat_stop: HWND,

    // Settings controls
    chk_ai_enabled: HWND,
    lbl_ai_backend: HWND,
    cmb_ai_backend: HWND,
    lbl_hymt_quant: HWND,
    cmb_hymt_quant: HWND,
    btn_hymt_check: HWND,
    btn_hymt_run: HWND,
    btn_hymt_stop: HWND,
    lbl_hymt_status: HWND,
    lbl_ai_url: HWND,
    edit_ai_url: HWND,
    lbl_ai_key: HWND,
    edit_ai_key: HWND,
    lbl_ai_model: HWND,
    edit_ai_model: HWND,
    btn_ai_discover: HWND,
    lbl_ai_prompt: HWND,
    edit_ai_prompt: HWND,

    lbl_chat_cfg: HWND,
    cmb_chat_backend: HWND,
    edit_chat_url: HWND,
    edit_chat_key: HWND,
    edit_chat_model: HWND,
    btn_chat_copy_trans: HWND,

    lbl_hk_header: HWND,
    edit_hk_toggle: HWND,
    edit_hk_ocr: HWND,
    edit_hk_chat: HWND,
    edit_hk_sel_trans: HWND,
    edit_hk_sel_chat: HWND,
    btn_hk_reset: HWND,

    lbl_opacity: HWND,
    cmb_opacity: HWND,
    chk_ocr_newline: HWND,
    chk_autostart: HWND,
    btn_save_settings: HWND,
    lbl_settings_status: HWND,
}

thread_local! {
    static WIN_CTX: RefCell<Option<Win32AppState>> = const { RefCell::new(None) };
}

fn with_state<F: FnOnce(&mut Win32AppState)>(f: F) {
    WIN_CTX.with(|cell| {
        if let Ok(mut borrow) = cell.try_borrow_mut() {
            if let Some(st) = borrow.as_mut() {
                f(st);
            }
        }
    });
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("初始化 Tokio 异步运行时失败");

    let settings = Arc::new(SettingsStore::open_default());
    let history = Arc::new(HistoryStore::open_default());
    let hotkey_mgr = GlobalHotKeyManager::new().expect("初始化全局快捷键管理器失败");
    let (ui_tx, ui_rx) = mpsc::unbounded_channel();

    let pane_layout = settings.pane_layout();
    let ai_enabled = settings.ai_translation_for_request().enabled;
    let win_w = pane_layout.width as i32;
    let win_h = pane_layout.translate_window_height(ai_enabled) as i32;

    unsafe {
        let hinstance = GetModuleHandleW(ptr::null());
        let class_name = wide_null("TTreePureRustWin32MainClass");
        let bg_brush = CreateSolidBrush(rgb(24, 26, 32));
        let surface_brush = CreateSolidBrush(rgb(34, 37, 46));

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: main_wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: bg_brush,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
        };
        RegisterClassExW(&wc);

        let font_face = wide_null("Microsoft YaHei UI");
        let font_regular = CreateFontW(
            -14,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            font_face.as_ptr(),
        );
        let font_bold = CreateFontW(
            -14,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            font_face.as_ptr(),
        );

        let title = wide_null("TTREE");
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN,
            260,
            180,
            win_w,
            win_h,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            panic!("创建 Windows 主窗口失败");
        }

        // 启用 Win11 原生深色标题栏、圆角与云母材质
        let dark_mode: u32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const u32 as *const c_void,
            4,
        );
        let corner_round: u32 = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner_round as *const u32 as *const c_void,
            4,
        );
        let mica_backdrop: u32 = DWMSBT_MAINWINDOW_MICA;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &mica_backdrop as *const u32 as *const c_void,
            4,
        );

        let opacity = settings.appearance().panel_opacity;
        let alpha = (opacity.clamp(PANEL_OPACITY_MIN, PANEL_OPACITY_MAX) * 255.0).round() as u8;
        SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);

        let convs = history.read_conversations().unwrap_or_default();
        let active_conv_idx = convs.len().saturating_sub(1);
        let current_messages = convs
            .get(active_conv_idx)
            .map(|c| c.messages.clone())
            .unwrap_or_default();

        let mut app_state = Win32AppState {
            rt: rt.handle().clone(),
            ui_tx,
            ui_rx,
            settings,
            history,
            hotkey_mgr,
            registered_hotkeys: Vec::new(),
            parsed_shortcuts: ParsedShortcuts::default(),
            hwnd,
            font_regular,
            font_bold,
            brush_bg: bg_brush,
            brush_surface: surface_brush,
            active_tab: TabMode::Translate,
            pinned: false,
            pin_before_settings: false,
            ocr_in_flight: false,
            suppress_source_change: false,
            source_lang_idx: 0,
            target_lang_idx: 0,
            pane_layout,
            dragging_splitter: None,
            translate_seq: 0,
            web_cancel: CancelSlot::new(),
            ai_cancel: CancelSlot::new(),
            ai_stream_buf: String::new(),
            web_result_text: String::new(),
            chat_seq: 0,
            chat_cancel: CancelSlot::new(),
            chat_streaming: false,
            conversations: convs,
            active_conv_idx,
            current_messages,
            streaming_reply: String::new(),
            hymt_cancel: CancelSlot::new(),
            pending_auto_translate: None,
            pending_resize_save: None,
            btn_tab_trans: ptr::null_mut(),
            btn_tab_chat: ptr::null_mut(),
            btn_tab_settings: ptr::null_mut(),
            btn_ocr: ptr::null_mut(),
            btn_pin: ptr::null_mut(),
            edit_source: ptr::null_mut(),
            cmb_source_lang: ptr::null_mut(),
            btn_swap_lang: ptr::null_mut(),
            cmb_target_lang: ptr::null_mut(),
            btn_trans_now: ptr::null_mut(),
            btn_clear_trans: ptr::null_mut(),
            lbl_web_header: ptr::null_mut(),
            btn_copy_web: ptr::null_mut(),
            edit_web_result: ptr::null_mut(),
            lbl_ai_header: ptr::null_mut(),
            btn_copy_ai: ptr::null_mut(),
            edit_ai_result: ptr::null_mut(),
            cmb_chat_conv: ptr::null_mut(),
            btn_chat_new: ptr::null_mut(),
            btn_chat_copy: ptr::null_mut(),
            btn_chat_clear: ptr::null_mut(),
            edit_chat_history: ptr::null_mut(),
            edit_chat_input: ptr::null_mut(),
            btn_chat_send: ptr::null_mut(),
            btn_chat_stop: ptr::null_mut(),
            chk_ai_enabled: ptr::null_mut(),
            lbl_ai_backend: ptr::null_mut(),
            cmb_ai_backend: ptr::null_mut(),
            lbl_hymt_quant: ptr::null_mut(),
            cmb_hymt_quant: ptr::null_mut(),
            btn_hymt_check: ptr::null_mut(),
            btn_hymt_run: ptr::null_mut(),
            btn_hymt_stop: ptr::null_mut(),
            lbl_hymt_status: ptr::null_mut(),
            lbl_ai_url: ptr::null_mut(),
            edit_ai_url: ptr::null_mut(),
            lbl_ai_key: ptr::null_mut(),
            edit_ai_key: ptr::null_mut(),
            lbl_ai_model: ptr::null_mut(),
            edit_ai_model: ptr::null_mut(),
            btn_ai_discover: ptr::null_mut(),
            lbl_ai_prompt: ptr::null_mut(),
            edit_ai_prompt: ptr::null_mut(),
            lbl_chat_cfg: ptr::null_mut(),
            cmb_chat_backend: ptr::null_mut(),
            edit_chat_url: ptr::null_mut(),
            edit_chat_key: ptr::null_mut(),
            edit_chat_model: ptr::null_mut(),
            btn_chat_copy_trans: ptr::null_mut(),
            lbl_hk_header: ptr::null_mut(),
            edit_hk_toggle: ptr::null_mut(),
            edit_hk_ocr: ptr::null_mut(),
            edit_hk_chat: ptr::null_mut(),
            edit_hk_sel_trans: ptr::null_mut(),
            edit_hk_sel_chat: ptr::null_mut(),
            btn_hk_reset: ptr::null_mut(),
            lbl_opacity: ptr::null_mut(),
            cmb_opacity: ptr::null_mut(),
            chk_ocr_newline: ptr::null_mut(),
            chk_autostart: ptr::null_mut(),
            btn_save_settings: ptr::null_mut(),
            lbl_settings_status: ptr::null_mut(),
        };

        app_state.build_child_controls(hinstance);
        let sc = app_state.settings.shortcuts();
        let _ = app_state.apply_shortcuts(&sc);
        app_state.populate_settings_fields();
        app_state.refresh_chat_controls();
        app_state.relayout_children();

        install_tray_icon(hwnd);
        SetTimer(hwnd, TIMER_ID_POLL, 25, ptr::null());

        if std::env::args().any(|a| a == "--show") {
            app_state.show_and_focus();
        }

        WIN_CTX.with(|cell| {
            *cell.borrow_mut() = Some(app_state);
        });

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        remove_tray_icon(hwnd);
        hy_mt_manager::stop_server();
    }
}

unsafe fn install_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = LoadIconW(ptr::null_mut(), IDI_APPLICATION);
    let tip = wide_null("TTREE AI 翻译与 OCR");
    let copy_len = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);
    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn show_tray_context_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let add = |id: usize, text: &str| {
        let w = wide_null(text);
        AppendMenuW(menu, MF_STRING, id, w.as_ptr());
    };
    add(ID_TRAY_TRANSLATE, "翻译窗口");
    add(ID_TRAY_OCR, "截图 OCR 翻译");
    add(ID_TRAY_CHAT, "AI 对话");
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    add(ID_TRAY_SETTINGS, "偏好设置…");
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    add(ID_TRAY_QUIT, "退出 TTREE");

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    );
    DestroyMenu(menu);
}

impl Win32AppState {
    unsafe fn create_ctrl(
        &self,
        hinstance: HINSTANCE,
        class: &str,
        title: &str,
        style: u32,
        id: usize,
        bold: bool,
    ) -> HWND {
        let cls = wide_null(class);
        let txt = wide_null(title);
        let ctrl = CreateWindowExW(
            0,
            cls.as_ptr(),
            txt.as_ptr(),
            WS_CHILD | style,
            0,
            0,
            10,
            10,
            self.hwnd,
            id as HMENU,
            hinstance,
            ptr::null_mut(),
        );
        let font = if bold {
            self.font_bold
        } else {
            self.font_regular
        };
        SendMessageW(ctrl, WM_SETFONT, font as WPARAM, 1);
        ctrl
    }

    unsafe fn build_child_controls(&mut self, hinst: HINSTANCE) {
        // 1. 顶部导航栏
        self.btn_tab_trans =
            self.create_ctrl(hinst, "BUTTON", "● 翻译", BS_PUSHBUTTON, ID_BTN_TAB_TRANS, true);
        self.btn_tab_chat =
            self.create_ctrl(hinst, "BUTTON", "AI 对话", BS_PUSHBUTTON, ID_BTN_TAB_CHAT, false);
        self.btn_ocr =
            self.create_ctrl(hinst, "BUTTON", "截图 OCR", BS_PUSHBUTTON, ID_BTN_OCR, false);
        self.btn_pin =
            self.create_ctrl(hinst, "BUTTON", "置顶: 关", BS_PUSHBUTTON, ID_BTN_PIN, false);
        self.btn_tab_settings =
            self.create_ctrl(hinst, "BUTTON", "设置", BS_PUSHBUTTON, ID_BTN_TAB_SETTINGS, false);

        // 2. 翻译面板控件
        self.edit_source = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN,
            ID_EDIT_SOURCE,
            false,
        );
        self.cmb_source_lang = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            ID_CMB_SOURCE_LANG,
            false,
        );
        for lang in SOURCE_LANGUAGES {
            let w = wide_null(lang.label);
            SendMessageW(self.cmb_source_lang, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
        }
        SendMessageW(self.cmb_source_lang, CB_SETCURSEL, 0, 0);

        self.btn_swap_lang =
            self.create_ctrl(hinst, "BUTTON", "⇄", BS_PUSHBUTTON, ID_BTN_SWAP_LANG, true);

        self.cmb_target_lang = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            ID_CMB_TARGET_LANG,
            false,
        );
        for lang in TARGET_LANGUAGES {
            let w = wide_null(lang.label);
            SendMessageW(self.cmb_target_lang, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
        }
        SendMessageW(self.cmb_target_lang, CB_SETCURSEL, 0, 0);

        self.btn_trans_now =
            self.create_ctrl(hinst, "BUTTON", "翻译", BS_PUSHBUTTON, ID_BTN_TRANS_NOW, true);
        self.btn_clear_trans =
            self.create_ctrl(hinst, "BUTTON", "清空", BS_PUSHBUTTON, ID_BTN_CLEAR_TRANS, false);

        self.lbl_web_header =
            self.create_ctrl(hinst, "STATIC", "Google / Bing 极速译文", 0, 0, true);
        self.btn_copy_web =
            self.create_ctrl(hinst, "BUTTON", "复制", BS_PUSHBUTTON, ID_BTN_COPY_WEB, false);
        self.edit_web_result = self.create_ctrl(
            hinst,
            "EDIT",
            "译文显示在这里",
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            ID_EDIT_WEB_RESULT,
            false,
        );

        self.lbl_ai_header = self.create_ctrl(hinst, "STATIC", "AI 深度译文", 0, 0, true);
        self.btn_copy_ai =
            self.create_ctrl(hinst, "BUTTON", "复制", BS_PUSHBUTTON, ID_BTN_COPY_AI, false);
        self.edit_ai_result = self.create_ctrl(
            hinst,
            "EDIT",
            "AI 译文显示在这里",
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            ID_EDIT_AI_RESULT,
            false,
        );

        // 3. AI 对话面板控件
        self.cmb_chat_conv = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            ID_CMB_CHAT_CONV,
            false,
        );
        self.btn_chat_new =
            self.create_ctrl(hinst, "BUTTON", "+ 新对话", BS_PUSHBUTTON, ID_BTN_CHAT_NEW, false);
        self.btn_chat_copy =
            self.create_ctrl(hinst, "BUTTON", "复制回复", BS_PUSHBUTTON, ID_BTN_CHAT_COPY, false);
        self.btn_chat_clear =
            self.create_ctrl(hinst, "BUTTON", "清空历史", BS_PUSHBUTTON, ID_BTN_CHAT_CLEAR, false);
        self.edit_chat_history = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            ID_EDIT_CHAT_HISTORY,
            false,
        );
        self.edit_chat_input = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN,
            ID_EDIT_CHAT_INPUT,
            false,
        );
        self.btn_chat_send =
            self.create_ctrl(hinst, "BUTTON", "发送", BS_PUSHBUTTON, ID_BTN_CHAT_SEND, true);
        self.btn_chat_stop =
            self.create_ctrl(hinst, "BUTTON", "停止", BS_PUSHBUTTON, ID_BTN_CHAT_STOP, false);

        // 4. 设置面板控件
        self.chk_ai_enabled = self.create_ctrl(
            hinst,
            "BUTTON",
            "启用 AI 深度翻译分栏",
            BS_AUTOCHECKBOX,
            ID_CHK_AI_ENABLED,
            true,
        );
        self.lbl_ai_backend = self.create_ctrl(hinst, "STATIC", "AI 翻译后端:", 0, 0, false);
        self.cmb_ai_backend = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            ID_CMB_AI_BACKEND,
            false,
        );
        for b in AiBackendKind::ALL {
            let w = wide_null(b.label());
            SendMessageW(self.cmb_ai_backend, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
        }

        self.lbl_hymt_quant =
            self.create_ctrl(hinst, "STATIC", "混元 HY-MT1.5 精度:", 0, 0, false);
        self.cmb_hymt_quant = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            ID_CMB_HYMT_QUANT,
            false,
        );
        for q in HyMtQuant::ALL {
            let w = wide_null(q.label());
            SendMessageW(self.cmb_hymt_quant, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
        }
        self.btn_hymt_check =
            self.create_ctrl(hinst, "BUTTON", "硬件检测", BS_PUSHBUTTON, ID_BTN_HYMT_CHECK, false);
        self.btn_hymt_run = self.create_ctrl(
            hinst,
            "BUTTON",
            "一键下载并运行",
            BS_PUSHBUTTON,
            ID_BTN_HYMT_RUN,
            true,
        );
        self.btn_hymt_stop =
            self.create_ctrl(hinst, "BUTTON", "停止服务", BS_PUSHBUTTON, ID_BTN_HYMT_STOP, false);
        self.lbl_hymt_status =
            self.create_ctrl(hinst, "STATIC", "内置 HY-MT1.5-1.8B 就绪状态: 待检测", 0, 0, false);

        self.lbl_ai_url = self.create_ctrl(hinst, "STATIC", "翻译接口 URL:", 0, 0, false);
        self.edit_ai_url = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_AI_URL,
            false,
        );
        self.lbl_ai_key = self.create_ctrl(hinst, "STATIC", "翻译 API Key:", 0, 0, false);
        self.edit_ai_key = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL | ES_PASSWORD,
            ID_EDIT_AI_KEY,
            false,
        );
        self.lbl_ai_model = self.create_ctrl(hinst, "STATIC", "翻译模型名称:", 0, 0, false);
        self.edit_ai_model = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_AI_MODEL,
            false,
        );
        self.btn_ai_discover = self.create_ctrl(
            hinst,
            "BUTTON",
            "探测本地",
            BS_PUSHBUTTON,
            ID_BTN_AI_DISCOVER,
            false,
        );
        self.lbl_ai_prompt = self.create_ctrl(hinst, "STATIC", "翻译提示词:", 0, 0, false);
        self.edit_ai_prompt = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_AI_PROMPT,
            false,
        );

        self.lbl_chat_cfg =
            self.create_ctrl(hinst, "STATIC", "AI 对话后端 / URL / Key / 模型:", 0, 0, true);
        self.cmb_chat_backend = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            ID_CMB_CHAT_BACKEND,
            false,
        );
        for b in AiBackendKind::ALL {
            let w = wide_null(b.label());
            SendMessageW(self.cmb_chat_backend, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
        }
        self.btn_chat_copy_trans = self.create_ctrl(
            hinst,
            "BUTTON",
            "同翻译配置",
            BS_PUSHBUTTON,
            ID_BTN_CHAT_COPY_TRANS,
            false,
        );
        self.edit_chat_url = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_CHAT_URL,
            false,
        );
        self.edit_chat_key = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL | ES_PASSWORD,
            ID_EDIT_CHAT_KEY,
            false,
        );
        self.edit_chat_model = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_CHAT_MODEL,
            false,
        );

        self.lbl_hk_header = self.create_ctrl(
            hinst,
            "STATIC",
            "全局热键 (呼出 / OCR / 对话 / 划词翻译 / 划词对话):",
            0,
            0,
            true,
        );
        self.edit_hk_toggle = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_HK_TOGGLE,
            false,
        );
        self.edit_hk_ocr = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_HK_OCR,
            false,
        );
        self.edit_hk_chat = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_HK_CHAT,
            false,
        );
        self.edit_hk_sel_trans = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_HK_SEL_TRANS,
            false,
        );
        self.edit_hk_sel_chat = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_HK_SEL_CHAT,
            false,
        );
        self.btn_hk_reset = self.create_ctrl(
            hinst,
            "BUTTON",
            "默认快捷键",
            BS_PUSHBUTTON,
            ID_BTN_HK_RESET,
            false,
        );

        self.lbl_opacity = self.create_ctrl(hinst, "STATIC", "窗口不透明度:", 0, 0, false);
        self.cmb_opacity = self.create_ctrl(
            hinst,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST | CBS_HASSTRINGS,
            ID_CMB_OPACITY,
            false,
        );
        for pct in ["100%", "90%", "80%", "70%", "60%", "50%"] {
            let w = wide_null(pct);
            SendMessageW(self.cmb_opacity, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
        }

        self.chk_ocr_newline = self.create_ctrl(
            hinst,
            "BUTTON",
            "OCR 自动合并多行换行",
            BS_AUTOCHECKBOX,
            ID_CHK_OCR_NEWLINE,
            false,
        );
        self.chk_autostart = self.create_ctrl(
            hinst,
            "BUTTON",
            "开机自动启动",
            BS_AUTOCHECKBOX,
            ID_CHK_AUTOSTART,
            false,
        );
        self.btn_save_settings = self.create_ctrl(
            hinst,
            "BUTTON",
            "保存设置",
            BS_PUSHBUTTON,
            ID_BTN_SAVE_SETTINGS,
            true,
        );
        self.lbl_settings_status = self.create_ctrl(hinst, "STATIC", "", 0, 0, false);
    }

    unsafe fn relayout_children(&self) {
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: 480,
            bottom: 520,
        };
        GetClientRect(self.hwnd, &mut rc);
        let cw = (rc.right - rc.left).max(360);
        let ch = (rc.bottom - rc.top).max(300);
        let pad = 12;
        let inner_w = cw - pad * 2;

        // 顶部栏布局
        let top_y = 8;
        let btn_h = 28;
        MoveWindow(self.btn_tab_trans, pad, top_y, 76, btn_h, 1);
        MoveWindow(self.btn_tab_chat, pad + 82, top_y, 76, btn_h, 1);
        MoveWindow(self.btn_ocr, cw - pad - 232, top_y, 82, btn_h, 1);
        MoveWindow(self.btn_pin, cw - pad - 144, top_y, 76, btn_h, 1);
        MoveWindow(self.btn_tab_settings, cw - pad - 62, top_y, 62, btn_h, 1);

        let trans_ctrls = [
            self.edit_source,
            self.cmb_source_lang,
            self.btn_swap_lang,
            self.cmb_target_lang,
            self.btn_trans_now,
            self.btn_clear_trans,
            self.lbl_web_header,
            self.btn_copy_web,
            self.edit_web_result,
            self.lbl_ai_header,
            self.btn_copy_ai,
            self.edit_ai_result,
        ];
        let chat_ctrls = [
            self.cmb_chat_conv,
            self.btn_chat_new,
            self.btn_chat_copy,
            self.btn_chat_clear,
            self.edit_chat_history,
            self.edit_chat_input,
            self.btn_chat_send,
            self.btn_chat_stop,
        ];
        let settings_ctrls = [
            self.chk_ai_enabled,
            self.lbl_ai_backend,
            self.cmb_ai_backend,
            self.lbl_hymt_quant,
            self.cmb_hymt_quant,
            self.btn_hymt_check,
            self.btn_hymt_run,
            self.btn_hymt_stop,
            self.lbl_hymt_status,
            self.lbl_ai_url,
            self.edit_ai_url,
            self.lbl_ai_key,
            self.edit_ai_key,
            self.lbl_ai_model,
            self.edit_ai_model,
            self.btn_ai_discover,
            self.lbl_ai_prompt,
            self.edit_ai_prompt,
            self.lbl_chat_cfg,
            self.cmb_chat_backend,
            self.btn_chat_copy_trans,
            self.edit_chat_url,
            self.edit_chat_key,
            self.edit_chat_model,
            self.lbl_hk_header,
            self.edit_hk_toggle,
            self.edit_hk_ocr,
            self.edit_hk_chat,
            self.edit_hk_sel_trans,
            self.edit_hk_sel_chat,
            self.btn_hk_reset,
            self.lbl_opacity,
            self.cmb_opacity,
            self.chk_ocr_newline,
            self.chk_autostart,
            self.btn_save_settings,
            self.lbl_settings_status,
        ];

        for &c in &trans_ctrls {
            ShowWindow(
                c,
                if self.active_tab == TabMode::Translate {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        for &c in &chat_ctrls {
            ShowWindow(
                c,
                if self.active_tab == TabMode::Chat {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        for &c in &settings_ctrls {
            ShowWindow(
                c,
                if self.active_tab == TabMode::Settings {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }

        let content_top = TOP_BAR_HEIGHT as i32;
        match self.active_tab {
            TabMode::Translate => {
                let ai_enabled = self.settings.ai_translation_for_request().enabled;
                let src_h = self.pane_layout.source_height as i32;
                let lang_h = self.pane_layout.lang_height as i32;
                let mut y = content_top;

                MoveWindow(self.edit_source, pad, y, inner_w, src_h, 1);
                y += src_h + 6;

                let lang_y = y + (lang_h - 26).max(0) / 2;
                let cmb_w = ((inner_w - 172) / 2).max(90);
                MoveWindow(self.cmb_source_lang, pad, lang_y, cmb_w, 220, 1);
                MoveWindow(self.btn_swap_lang, pad + cmb_w + 4, lang_y, 32, 26, 1);
                MoveWindow(
                    self.cmb_target_lang,
                    pad + cmb_w + 40,
                    lang_y,
                    cmb_w,
                    220,
                    1,
                );
                MoveWindow(self.btn_trans_now, cw - pad - 122, lang_y, 60, 26, 1);
                MoveWindow(self.btn_clear_trans, cw - pad - 58, lang_y, 58, 26, 1);
                y += lang_h + 6;

                let remaining_h = (ch - y - pad).max(120);
                let web_total_h = if ai_enabled {
                    (self.pane_layout.web_height as i32).clamp(76, (remaining_h - 86).max(76))
                } else {
                    remaining_h
                };

                MoveWindow(self.lbl_web_header, pad, y, inner_w - 64, 22, 1);
                MoveWindow(self.btn_copy_web, cw - pad - 58, y - 2, 58, 22, 1);
                MoveWindow(
                    self.edit_web_result,
                    pad,
                    y + 24,
                    inner_w,
                    (web_total_h - 24).max(48),
                    1,
                );
                y += web_total_h + 8;

                if ai_enabled {
                    let ai_total_h = (ch - y - pad).max(76);
                    ShowWindow(self.lbl_ai_header, SW_SHOW);
                    ShowWindow(self.btn_copy_ai, SW_SHOW);
                    ShowWindow(self.edit_ai_result, SW_SHOW);
                    MoveWindow(self.lbl_ai_header, pad, y, inner_w - 64, 22, 1);
                    MoveWindow(self.btn_copy_ai, cw - pad - 58, y - 2, 58, 22, 1);
                    MoveWindow(
                        self.edit_ai_result,
                        pad,
                        y + 24,
                        inner_w,
                        (ai_total_h - 24).max(48),
                        1,
                    );
                } else {
                    ShowWindow(self.lbl_ai_header, SW_HIDE);
                    ShowWindow(self.btn_copy_ai, SW_HIDE);
                    ShowWindow(self.edit_ai_result, SW_HIDE);
                }
            }
            TabMode::Chat => {
                let mut y = content_top;
                let cmb_w = (inner_w - 222).max(110);
                MoveWindow(self.cmb_chat_conv, pad, y, cmb_w, 220, 1);
                MoveWindow(self.btn_chat_new, pad + cmb_w + 6, y, 68, 26, 1);
                MoveWindow(self.btn_chat_copy, pad + cmb_w + 78, y, 68, 26, 1);
                MoveWindow(self.btn_chat_clear, pad + cmb_w + 150, y, 72, 26, 1);
                y += 32;

                let input_h = (self.pane_layout.chat_input_height as i32).clamp(68, 200);
                let hist_h = (ch - y - input_h - 46).max(100);
                MoveWindow(self.edit_chat_history, pad, y, inner_w, hist_h, 1);
                y += hist_h + 6;

                MoveWindow(self.edit_chat_input, pad, y, inner_w, input_h, 1);
                y += input_h + 6;
                MoveWindow(self.btn_chat_send, cw - pad - 132, y, 64, 28, 1);
                MoveWindow(self.btn_chat_stop, cw - pad - 64, y, 64, 28, 1);
            }
            TabMode::Settings => {
                let mut y = content_top;
                let row_h = 24;
                let gap = 5;
                let lbl_w = 132;
                let field_x = pad + lbl_w;
                let field_w = (inner_w - lbl_w).max(160);

                MoveWindow(self.chk_ai_enabled, pad, y, inner_w, row_h, 1);
                y += row_h + gap;

                MoveWindow(self.lbl_ai_backend, pad, y + 3, lbl_w, row_h, 1);
                MoveWindow(self.cmb_ai_backend, field_x, y, field_w, 200, 1);
                y += row_h + gap;

                MoveWindow(self.lbl_hymt_quant, pad, y + 3, lbl_w, row_h, 1);
                MoveWindow(self.cmb_hymt_quant, field_x, y, field_w, 220, 1);
                y += row_h + gap;

                MoveWindow(self.btn_hymt_check, pad, y, 86, row_h, 1);
                MoveWindow(self.btn_hymt_run, pad + 92, y, 132, row_h, 1);
                MoveWindow(self.btn_hymt_stop, pad + 230, y, 86, row_h, 1);
                y += row_h + 3;
                MoveWindow(self.lbl_hymt_status, pad, y, inner_w, 20, 1);
                y += 22;

                MoveWindow(self.lbl_ai_url, pad, y + 3, lbl_w, row_h, 1);
                MoveWindow(self.edit_ai_url, field_x, y, field_w, row_h, 1);
                y += row_h + gap;

                MoveWindow(self.lbl_ai_key, pad, y + 3, lbl_w, row_h, 1);
                MoveWindow(self.edit_ai_key, field_x, y, field_w, row_h, 1);
                y += row_h + gap;

                MoveWindow(self.lbl_ai_model, pad, y + 3, lbl_w, row_h, 1);
                MoveWindow(self.edit_ai_model, field_x, y, field_w - 76, row_h, 1);
                MoveWindow(self.btn_ai_discover, cw - pad - 72, y, 72, row_h, 1);
                y += row_h + gap;

                MoveWindow(self.lbl_ai_prompt, pad, y + 3, lbl_w, row_h, 1);
                MoveWindow(self.edit_ai_prompt, field_x, y, field_w, row_h, 1);
                y += row_h + 8;

                MoveWindow(self.lbl_chat_cfg, pad, y + 3, inner_w - 90, row_h, 1);
                MoveWindow(self.btn_chat_copy_trans, cw - pad - 86, y, 86, row_h, 1);
                y += row_h + 4;

                let half_w = (inner_w - 6) / 2;
                MoveWindow(self.cmb_chat_backend, pad, y, half_w, 200, 1);
                MoveWindow(self.edit_chat_model, pad + half_w + 6, y, half_w, row_h, 1);
                y += row_h + 4;
                MoveWindow(self.edit_chat_url, pad, y, half_w, row_h, 1);
                MoveWindow(self.edit_chat_key, pad + half_w + 6, y, half_w, row_h, 1);
                y += row_h + 8;

                MoveWindow(self.lbl_hk_header, pad, y + 3, inner_w - 88, row_h, 1);
                MoveWindow(self.btn_hk_reset, cw - pad - 84, y, 84, row_h, 1);
                y += row_h + 4;

                let third_w = (inner_w - 8) / 3;
                MoveWindow(self.edit_hk_toggle, pad, y, third_w, row_h, 1);
                MoveWindow(self.edit_hk_ocr, pad + third_w + 4, y, third_w, row_h, 1);
                MoveWindow(
                    self.edit_hk_chat,
                    pad + (third_w + 4) * 2,
                    y,
                    third_w,
                    row_h,
                    1,
                );
                y += row_h + 4;
                MoveWindow(self.edit_hk_sel_trans, pad, y, half_w, row_h, 1);
                MoveWindow(self.edit_hk_sel_chat, pad + half_w + 6, y, half_w, row_h, 1);
                y += row_h + 8;

                MoveWindow(self.lbl_opacity, pad, y + 3, 98, row_h, 1);
                MoveWindow(self.cmb_opacity, pad + 100, y, 76, 180, 1);
                MoveWindow(self.chk_ocr_newline, pad + 186, y, 165, row_h, 1);
                MoveWindow(self.chk_autostart, pad + 356, y, 115, row_h, 1);
                y += row_h + 8;

                MoveWindow(self.btn_save_settings, pad, y, 96, 28, 1);
                MoveWindow(self.lbl_settings_status, pad + 104, y + 4, inner_w - 104, 24, 1);
            }
        }
        InvalidateRect(self.hwnd, ptr::null(), 1);
    }

    unsafe fn switch_tab(&mut self, tab: TabMode) {
        if self.active_tab == tab {
            return;
        }
        if self.active_tab == TabMode::Settings && tab != TabMode::Settings {
            self.set_pinned(self.pin_before_settings);
        } else if tab == TabMode::Settings && self.active_tab != TabMode::Settings {
            self.pin_before_settings = self.pinned;
            self.set_pinned(true);
            self.populate_settings_fields();
        }
        self.active_tab = tab;

        set_hwnd_text(
            self.btn_tab_trans,
            if tab == TabMode::Translate {
                "● 翻译"
            } else {
                "翻译"
            },
        );
        set_hwnd_text(
            self.btn_tab_chat,
            if tab == TabMode::Chat {
                "● AI 对话"
            } else {
                "AI 对话"
            },
        );
        set_hwnd_text(
            self.btn_tab_settings,
            if tab == TabMode::Settings {
                "● 设置"
            } else {
                "设置"
            },
        );

        self.relayout_children();
        match tab {
            TabMode::Translate => {
                SetFocus(self.edit_source);
            }
            TabMode::Chat => {
                SetFocus(self.edit_chat_input);
            }
            TabMode::Settings => {}
        }
    }

    unsafe fn set_pinned(&mut self, pin: bool) {
        self.pinned = pin;
        let insert_after = if pin { HWND_TOPMOST } else { HWND_NOTOPMOST };
        SetWindowPos(
            self.hwnd,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        set_hwnd_text(self.btn_pin, if pin { "置顶: 开" } else { "置顶: 关" });
    }

    unsafe fn show_and_focus(&mut self) {
        ShowWindow(self.hwnd, SW_RESTORE);
        ShowWindow(self.hwnd, SW_SHOW);
        SetForegroundWindow(self.hwnd);
        match self.active_tab {
            TabMode::Translate => {
                SetFocus(self.edit_source);
            }
            TabMode::Chat => {
                SetFocus(self.edit_chat_input);
            }
            TabMode::Settings => {}
        }
    }

    unsafe fn hide_window(&mut self) {
        self.web_cancel.cancel_current();
        self.ai_cancel.cancel_current();
        ShowWindow(self.hwnd, SW_HIDE);
    }

    fn apply_shortcuts(&mut self, cfg: &ShortcutConfig) -> Result<(), String> {
        let parsed = parse_shortcut_config(cfg).map_err(|e| e.to_string())?;
        for hk in self.registered_hotkeys.drain(..) {
            let _ = self.hotkey_mgr.unregister(hk);
        }
        for hk in parsed.unique_hotkeys() {
            self.hotkey_mgr
                .register(hk)
                .map_err(|e| format!("注册全局快捷键失败: {e}"))?;
            self.registered_hotkeys.push(hk);
        }
        self.parsed_shortcuts = parsed;
        Ok(())
    }

    unsafe fn handle_hotkey_action(&mut self, action: ShortcutAction) {
        match action {
            ShortcutAction::Toggle => {
                if IsWindowVisible(self.hwnd) != 0 && !self.pinned {
                    self.hide_window();
                } else {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus();
                }
            }
            ShortcutAction::Ocr => {
                self.trigger_ocr();
            }
            ShortcutAction::AiDialog => {
                self.switch_tab(TabMode::Chat);
                self.show_and_focus();
            }
            ShortcutAction::SelectionTranslate => {
                if let Ok(Some(selected)) = capture_selected_text() {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus();
                    self.suppress_source_change = true;
                    set_hwnd_text(self.edit_source, &selected);
                    self.suppress_source_change = false;
                    self.start_translation();
                } else if IsWindowVisible(self.hwnd) != 0 && !self.pinned {
                    self.hide_window();
                } else {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus();
                }
            }
            ShortcutAction::SelectionAiDialog => {
                if let Ok(Some(selected)) = capture_selected_text() {
                    self.switch_tab(TabMode::Chat);
                    self.show_and_focus();
                    set_hwnd_text(self.edit_chat_input, &selected);
                    self.send_chat_from_input();
                } else {
                    self.switch_tab(TabMode::Chat);
                    self.show_and_focus();
                }
            }
        }
    }

    unsafe fn start_translation(&mut self) {
        self.pending_auto_translate = None;
        let text = get_hwnd_text(self.edit_source);
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            self.web_cancel.cancel_current();
            self.ai_cancel.cancel_current();
            set_hwnd_text(self.edit_web_result, "译文显示在这里");
            set_hwnd_text(self.edit_ai_result, "AI 译文显示在这里");
            return;
        }

        self.translate_seq = self.translate_seq.wrapping_add(1);
        let seq = self.translate_seq;

        let src_code = SOURCE_LANGUAGES
            .get(self.source_lang_idx)
            .map(|l| l.code)
            .unwrap_or("auto");
        let tgt_code = TARGET_LANGUAGES
            .get(self.target_lang_idx)
            .map(|l| l.code)
            .unwrap_or(ADAPTIVE_TARGET);
        let (resolved_src, resolved_tgt) = resolve_language_pair(src_code, tgt_code, &trimmed);

        set_hwnd_text(self.edit_web_result, "翻译中…");
        let web_token = self.web_cancel.next();
        let tx_web = self.ui_tx.clone();
        let text_for_web = trimmed.clone();
        let src_for_web = resolved_src.to_string();
        let tgt_for_web = resolved_tgt.to_string();
        self.rt.spawn(async move {
            let res =
                ttree_translate::translate_web(&text_for_web, &src_for_web, &tgt_for_web, &web_token)
                    .await;
            let _ = tx_web.send(UiEvent::WebTranslateDone { seq, result: res });
        });

        let ai_cfg = self.settings.ai_translation_for_request();
        if ai_cfg.enabled {
            self.ai_stream_buf.clear();
            set_hwnd_text(self.edit_ai_result, "AI 翻译中…");
            let ai_token = self.ai_cancel.next();
            let tx_ai = self.ui_tx.clone();
            let tx_ai_done = self.ui_tx.clone();
            let src_for_ai = resolved_src.to_string();
            let tgt_for_ai = resolved_tgt.to_string();
            self.rt.spawn(async move {
                let res = ttree_ai::stream_translate(
                    &ai_cfg,
                    &trimmed,
                    &src_for_ai,
                    &tgt_for_ai,
                    &ai_token,
                    |delta| {
                        let _ = tx_ai.send(UiEvent::AiTranslateDelta {
                            seq,
                            delta: delta.to_string(),
                        });
                    },
                )
                .await;
                let _ = tx_ai_done.send(UiEvent::AiTranslateDone { seq, result: res });
            });
        }
    }

    unsafe fn trigger_ocr(&mut self) {
        if self.ocr_in_flight {
            return;
        }
        self.ocr_in_flight = true;
        ShowWindow(self.hwnd, SW_HIDE);
        let tx = self.ui_tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            let res = ttree_ocr::capture_and_recognize();
            let _ = tx.send(UiEvent::OcrDone(res));
        });
    }

    unsafe fn refresh_chat_controls(&mut self) {
        SendMessageW(self.cmb_chat_conv, CB_RESETCONTENT, 0, 0);
        if self.conversations.is_empty() {
            let w = wide_null("当前新对话");
            SendMessageW(self.cmb_chat_conv, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
            SendMessageW(self.cmb_chat_conv, CB_SETCURSEL, 0, 0);
        } else {
            for (idx, conv) in self.conversations.iter().enumerate() {
                let title = if conv.summary.trim().is_empty() {
                    format!("对话 #{}", idx + 1)
                } else {
                    format!("#{}: {}", idx + 1, conv.summary)
                };
                let w = wide_null(&title);
                SendMessageW(self.cmb_chat_conv, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
            }
            let sel = self.active_conv_idx.min(self.conversations.len() - 1);
            SendMessageW(self.cmb_chat_conv, CB_SETCURSEL, sel, 0);
        }
        self.render_chat_transcript();
    }

    unsafe fn render_chat_transcript(&self) {
        let mut out = String::new();
        for msg in &self.current_messages {
            let role_label = if msg.role == "user" { "【我】" } else { "【AI】" };
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(role_label);
            out.push('\n');
            out.push_str(&msg.content);
        }
        if self.chat_streaming {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("【AI】\n");
            if self.streaming_reply.is_empty() {
                out.push_str("思考中…");
            } else {
                out.push_str(&self.streaming_reply);
            }
        }
        if out.is_empty() {
            out.push_str("在下方输入问题，开始多轮 AI 对话…");
        }
        set_hwnd_text(self.edit_chat_history, &out);
        let len = GetWindowTextLengthW(self.edit_chat_history) as usize;
        SendMessageW(self.edit_chat_history, EM_SETSEL, len, len as LPARAM);
        SendMessageW(self.edit_chat_history, EM_SCROLLCARET, 0, 0);
    }

    unsafe fn send_chat_from_input(&mut self) {
        if self.chat_streaming {
            return;
        }
        let text = get_hwnd_text(self.edit_chat_input).trim().to_string();
        if text.is_empty() {
            return;
        }
        set_hwnd_text(self.edit_chat_input, "");

        let user_msg = ChatMessage::user(text);
        self.current_messages.push(user_msg);
        self.streaming_reply.clear();
        self.chat_streaming = true;
        self.render_chat_transcript();

        self.chat_seq = self.chat_seq.wrapping_add(1);
        let seq = self.chat_seq;
        let token = self.chat_cancel.next();
        let cfg = self.settings.chat_ai_for_request();
        let context = recent_chat_context(&self.current_messages);
        let tx = self.ui_tx.clone();
        let tx_done = self.ui_tx.clone();

        self.rt.spawn(async move {
            let res = ttree_ai::stream_chat(&cfg, &context, &token, |delta| {
                let _ = tx.send(UiEvent::ChatDelta {
                    seq,
                    delta: delta.to_string(),
                });
            })
            .await;
            let _ = tx_done.send(UiEvent::ChatDone { seq, result: res });
        });
    }

    unsafe fn populate_settings_fields(&self) {
        let ai_cfg = self.settings.ai_translation_for_request();
        let chat_cfg = self.settings.chat_ai_for_request();
        let hk_cfg = self.settings.shortcuts();
        let app_cfg = self.settings.appearance();

        SendMessageW(
            self.chk_ai_enabled,
            BM_SETCHECK,
            if ai_cfg.enabled {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
        let b_idx = AiBackendKind::ALL
            .iter()
            .position(|&b| b == ai_cfg.backend)
            .unwrap_or(0);
        SendMessageW(self.cmb_ai_backend, CB_SETCURSEL, b_idx, 0);

        let q_idx = HyMtQuant::ALL
            .iter()
            .position(|&q| q == ai_cfg.hy_mt_quant)
            .unwrap_or(6);
        SendMessageW(self.cmb_hymt_quant, CB_SETCURSEL, q_idx, 0);

        set_hwnd_text(self.edit_ai_url, &ai_cfg.base_url);
        set_hwnd_text(self.edit_ai_key, &ai_cfg.api_key);
        set_hwnd_text(self.edit_ai_model, &ai_cfg.model);
        set_hwnd_text(self.edit_ai_prompt, &ai_cfg.translate_prompt);

        let cb_idx = AiBackendKind::ALL
            .iter()
            .position(|&b| b == chat_cfg.backend)
            .unwrap_or(0);
        SendMessageW(self.cmb_chat_backend, CB_SETCURSEL, cb_idx, 0);
        set_hwnd_text(self.edit_chat_url, &chat_cfg.base_url);
        set_hwnd_text(self.edit_chat_key, &chat_cfg.api_key);
        set_hwnd_text(self.edit_chat_model, &chat_cfg.model);

        set_hwnd_text(self.edit_hk_toggle, &hk_cfg.toggle);
        set_hwnd_text(self.edit_hk_ocr, &hk_cfg.ocr);
        set_hwnd_text(self.edit_hk_chat, &hk_cfg.ai_dialog);
        set_hwnd_text(self.edit_hk_sel_trans, &hk_cfg.selection_translate);
        set_hwnd_text(self.edit_hk_sel_chat, &hk_cfg.selection_ai_dialog);

        let op_idx = if app_cfg.panel_opacity >= 0.95 {
            0
        } else if app_cfg.panel_opacity >= 0.85 {
            1
        } else if app_cfg.panel_opacity >= 0.75 {
            2
        } else if app_cfg.panel_opacity >= 0.65 {
            3
        } else if app_cfg.panel_opacity >= 0.55 {
            4
        } else {
            5
        };
        SendMessageW(self.cmb_opacity, CB_SETCURSEL, op_idx, 0);

        SendMessageW(
            self.chk_ocr_newline,
            BM_SETCHECK,
            if app_cfg.ocr_strip_newlines {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
        SendMessageW(
            self.chk_autostart,
            BM_SETCHECK,
            if autostart::is_enabled() {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
    }

    unsafe fn save_settings_from_ui(&mut self) {
        let ai_enabled = SendMessageW(self.chk_ai_enabled, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        let b_idx = (SendMessageW(self.cmb_ai_backend, CB_GETCURSEL, 0, 0) as usize)
            .min(AiBackendKind::ALL.len() - 1);
        let q_idx = (SendMessageW(self.cmb_hymt_quant, CB_GETCURSEL, 0, 0) as usize)
            .min(HyMtQuant::ALL.len() - 1);

        let ai_cfg = AiTranslationConfig {
            enabled: ai_enabled,
            backend: AiBackendKind::ALL[b_idx],
            hy_mt_quant: HyMtQuant::ALL[q_idx],
            base_url: get_hwnd_text(self.edit_ai_url),
            api_key: get_hwnd_text(self.edit_ai_key),
            has_api_key: !get_hwnd_text(self.edit_ai_key).trim().is_empty(),
            clear_api_key: get_hwnd_text(self.edit_ai_key).trim().is_empty(),
            model: get_hwnd_text(self.edit_ai_model),
            gguf_model_path: String::new(),
            gguf_tokenizer_path: String::new(),
            translate_prompt: get_hwnd_text(self.edit_ai_prompt),
        };
        if let Err(e) = self.settings.save_ai_translation(&ai_cfg) {
            set_hwnd_text(self.lbl_settings_status, &format!("保存翻译设置失败: {e}"));
            return;
        }

        let cb_idx = (SendMessageW(self.cmb_chat_backend, CB_GETCURSEL, 0, 0) as usize)
            .min(AiBackendKind::ALL.len() - 1);
        let chat_cfg = ChatAiConfig {
            backend: AiBackendKind::ALL[cb_idx],
            hy_mt_quant: HyMtQuant::ALL[q_idx],
            base_url: get_hwnd_text(self.edit_chat_url),
            api_key: get_hwnd_text(self.edit_chat_key),
            has_api_key: !get_hwnd_text(self.edit_chat_key).trim().is_empty(),
            clear_api_key: get_hwnd_text(self.edit_chat_key).trim().is_empty(),
            model: get_hwnd_text(self.edit_chat_model),
            gguf_model_path: String::new(),
            gguf_tokenizer_path: String::new(),
            chat_prompt: self.settings.chat_ai_for_request().chat_prompt,
        };
        if let Err(e) = self.settings.save_chat_ai(&chat_cfg) {
            set_hwnd_text(self.lbl_settings_status, &format!("保存对话设置失败: {e}"));
            return;
        }

        let hk_cfg = ShortcutConfig {
            toggle: get_hwnd_text(self.edit_hk_toggle),
            ocr: get_hwnd_text(self.edit_hk_ocr),
            ai_dialog: get_hwnd_text(self.edit_hk_chat),
            selection_translate: get_hwnd_text(self.edit_hk_sel_trans),
            selection_ai_dialog: get_hwnd_text(self.edit_hk_sel_chat),
        };
        if let Err(e) = self.apply_shortcuts(&hk_cfg) {
            set_hwnd_text(self.lbl_settings_status, &e);
            return;
        }
        if let Err(e) = self.settings.save_shortcuts(&hk_cfg) {
            set_hwnd_text(self.lbl_settings_status, &format!("保存快捷键失败: {e}"));
            return;
        }

        let op_idx = SendMessageW(self.cmb_opacity, CB_GETCURSEL, 0, 0) as usize;
        let opacity = match op_idx {
            0 => 1.0,
            1 => 0.9,
            2 => 0.8,
            3 => 0.7,
            4 => 0.6,
            _ => 0.5,
        };
        let ocr_strip =
            SendMessageW(self.chk_ocr_newline, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        let app_cfg = AppearanceConfig {
            panel_opacity: opacity,
            ocr_strip_newlines: ocr_strip,
        };
        let _ = self.settings.save_appearance(app_cfg);
        let alpha = (opacity * 255.0).round() as u8;
        SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);

        let want_autostart =
            SendMessageW(self.chk_autostart, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        let _ = autostart::set_enabled(want_autostart);

        set_hwnd_text(self.lbl_settings_status, "✓ 设置已保存并立即生效");
    }

    unsafe fn handle_ui_event(&mut self, ev: UiEvent) {
        match ev {
            UiEvent::WebTranslateDone { seq, result } => {
                if seq != self.translate_seq {
                    return;
                }
                match result {
                    Ok(text) => {
                        self.web_result_text = text.clone();
                        set_hwnd_text(
                            self.edit_web_result,
                            if text.is_empty() {
                                "译文显示在这里"
                            } else {
                                &text
                            },
                        );
                    }
                    Err(err) => {
                        set_hwnd_text(self.edit_web_result, &format!("翻译失败: {err}"));
                    }
                }
            }
            UiEvent::AiTranslateDelta { seq, delta } => {
                if seq != self.translate_seq {
                    return;
                }
                self.ai_stream_buf.push_str(&delta);
                set_hwnd_text(self.edit_ai_result, &self.ai_stream_buf);
            }
            UiEvent::AiTranslateDone { seq, result } => {
                if seq != self.translate_seq {
                    return;
                }
                if let Err(err) = result {
                    if self.ai_stream_buf.is_empty() {
                        set_hwnd_text(self.edit_ai_result, &format!("AI 翻译失败: {err}"));
                    }
                }
            }
            UiEvent::ChatDelta { seq, delta } => {
                if seq != self.chat_seq {
                    return;
                }
                self.streaming_reply.push_str(&delta);
                self.render_chat_transcript();
            }
            UiEvent::ChatDone { seq, result } => {
                if seq != self.chat_seq {
                    return;
                }
                self.chat_streaming = false;
                match result {
                    Ok(()) => {
                        let reply = std::mem::take(&mut self.streaming_reply);
                        if !reply.trim().is_empty() {
                            if let Some(user_last) = self
                                .current_messages
                                .iter()
                                .rev()
                                .find(|m| m.role == "user")
                                .cloned()
                            {
                                let assistant_msg = ChatMessage::assistant(reply);
                                self.current_messages.push(assistant_msg.clone());
                                let conv_id = self
                                    .conversations
                                    .get(self.active_conv_idx)
                                    .map(|c| c.id);
                                let _ = self
                                    .history
                                    .append_round(conv_id, &[user_last, assistant_msg]);
                                self.conversations =
                                    self.history.read_conversations().unwrap_or_default();
                                self.active_conv_idx = self.conversations.len().saturating_sub(1);
                            }
                        }
                    }
                    Err(err) => {
                        self.current_messages
                            .push(ChatMessage::assistant(format!("请求失败: {err}")));
                    }
                }
                self.refresh_chat_controls();
            }
            UiEvent::OcrDone(res) => {
                self.ocr_in_flight = false;
                match res {
                    Ok(OcrOutcome::Cancelled) | Ok(OcrOutcome::Busy) => {}
                    Ok(OcrOutcome::EmptyText) => {
                        self.switch_tab(TabMode::Translate);
                        self.show_and_focus();
                        set_hwnd_text(
                            self.edit_web_result,
                            "所选截图区域未检测到可识别的文字，请重新框选。",
                        );
                    }
                    Ok(OcrOutcome::PermissionDenied(msg)) => {
                        self.switch_tab(TabMode::Translate);
                        self.show_and_focus();
                        set_hwnd_text(self.edit_web_result, &msg);
                    }
                    Ok(OcrOutcome::Text(raw_text)) => {
                        let text = if self.settings.appearance().ocr_strip_newlines {
                            normalize_ocr_newlines(&raw_text)
                        } else {
                            raw_text.trim().to_string()
                        };
                        self.switch_tab(TabMode::Translate);
                        self.show_and_focus();
                        self.suppress_source_change = true;
                        set_hwnd_text(self.edit_source, &text);
                        self.suppress_source_change = false;
                        self.start_translation();
                    }
                    Err(err) => {
                        self.switch_tab(TabMode::Translate);
                        self.show_and_focus();
                        set_hwnd_text(self.edit_web_result, &format!("截图 OCR 失败: {err}"));
                    }
                }
            }
            UiEvent::ModelsDiscovered(res) => match res {
                Ok(models) if !models.is_empty() => {
                    set_hwnd_text(self.edit_ai_model, &models[0]);
                    set_hwnd_text(
                        self.lbl_settings_status,
                        &format!("已发现 {} 个本地模型: {}", models.len(), models.join(", ")),
                    );
                }
                Ok(_) => {
                    set_hwnd_text(self.lbl_settings_status, "未探测到本地已加载模型");
                }
                Err(e) => {
                    set_hwnd_text(self.lbl_settings_status, &format!("探测模型失败: {e}"));
                }
            },
            UiEvent::HyMtProgress(prog) => {
                let mb_done = prog.downloaded_bytes as f64 / (1024.0 * 1024.0);
                let mb_total = (prog.total_bytes.max(1)) as f64 / (1024.0 * 1024.0);
                let pct = ((prog.downloaded_bytes as f64 / (prog.total_bytes.max(1) as f64))
                    * 100.0)
                    .clamp(0.0, 100.0);
                set_hwnd_text(
                    self.lbl_hymt_status,
                    &format!(
                        "{}: {:.1}% ({:.0}/{:.0} MB, {:.1} MB/s)",
                        prog.stage, pct, mb_done, mb_total, prog.speed_mbps
                    ),
                );
            }
            UiEvent::HyMtReady(res) => match res {
                Ok(url) => {
                    set_hwnd_text(
                        self.lbl_hymt_status,
                        &format!("✓ HY-MT1.5-1.8B 本地推理服务运行中 ({url})"),
                    );
                }
                Err(err) => {
                    set_hwnd_text(self.lbl_hymt_status, &format!("HY-MT1.5 启动失败: {err}"));
                }
            },
            UiEvent::HyMtCompatReady(rep) => {
                set_hwnd_text(
                    self.lbl_hymt_status,
                    &format!(
                        "{} | 内存 {:.1}GB | 磁盘可用 {:.1}GB | 推荐精度: {}",
                        rep.gpu_backend,
                        rep.total_ram_gb,
                        rep.free_disk_gb,
                        rep.recommended_quant.code()
                    ),
                );
            }
        }
    }

    unsafe fn poll_tick(&mut self) {
        while let Ok(hk_ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if hk_ev.state == HotKeyState::Pressed {
                if let Some(action) = self.parsed_shortcuts.action_for_id(hk_ev.id) {
                    self.handle_hotkey_action(action);
                }
            }
        }
        while let Ok(ui_ev) = self.ui_rx.try_recv() {
            self.handle_ui_event(ui_ev);
        }

        let now = Instant::now();
        if let Some(deadline) = self.pending_auto_translate {
            if now >= deadline {
                self.pending_auto_translate = None;
                self.start_translation();
            }
        }
        if let Some((deadline, w, _h)) = self.pending_resize_save {
            if now >= deadline {
                self.pending_resize_save = None;
                self.pane_layout.width = w.clamp(WINDOW_MIN_WIDTH, WINDOW_MAX_WIDTH);
                self.settings.save_pane_layout(self.pane_layout);
            }
        }
    }
}

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam == TIMER_ID_POLL {
                with_state(|st| st.poll_tick());
            }
            0
        }
        WM_ACTIVATE => {
            let active_state = wparam & 0xFFFF;
            if active_state == WA_INACTIVE {
                with_state(|st| {
                    if !st.pinned && !st.ocr_in_flight && st.active_tab != TabMode::Settings {
                        st.hide_window();
                    }
                });
            }
            0
        }
        WM_SIZE => {
            let w = (lparam as u32 & 0xFFFF) as i16 as i32;
            let h = ((lparam as u32 >> 16) & 0xFFFF) as i16 as i32;
            if w > 0 && h > 0 {
                with_state(|st| {
                    st.relayout_children();
                    st.pending_resize_save = Some((
                        Instant::now() + RESIZE_SAVE_DEBOUNCE,
                        w as f64,
                        h as f64,
                    ));
                });
            }
            0
        }
        WM_GETMINMAXINFO => {
            let mmi = lparam as *mut MINMAXINFO;
            if !mmi.is_null() {
                (*mmi).ptMinTrackSize.x = WINDOW_MIN_WIDTH as i32;
                (*mmi).ptMinTrackSize.y = WINDOW_MIN_HEIGHT as i32;
                (*mmi).ptMaxTrackSize.x = WINDOW_MAX_WIDTH as i32;
                (*mmi).ptMaxTrackSize.y = WINDOW_MAX_HEIGHT as i32;
            }
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let hdc = wparam as HDC;
            SetTextColor(hdc, rgb(235, 238, 245));
            SetBkColor(hdc, rgb(34, 37, 46));
            SetBkMode(hdc, OPAQUE);
            let mut brush = ptr::null_mut();
            with_state(|st| brush = st.brush_surface);
            brush as LRESULT
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let hdc = wparam as HDC;
            SetTextColor(hdc, rgb(218, 224, 236));
            SetBkColor(hdc, rgb(24, 26, 32));
            SetBkMode(hdc, TRANSPARENT);
            let mut brush = ptr::null_mut();
            with_state(|st| brush = st.brush_bg);
            brush as LRESULT
        }
        WM_LBUTTONDOWN => {
            let y = ((lparam as u32 >> 16) & 0xFFFF) as i16 as i32;
            with_state(|st| {
                if st.active_tab == TabMode::Translate {
                    let s1 = (TOP_BAR_HEIGHT + st.pane_layout.source_height) as i32;
                    let s2 = s1 + 6 + (st.pane_layout.lang_height as i32) + 6 + (st.pane_layout.web_height as i32);
                    if (y - s1).abs() <= 6 {
                        st.dragging_splitter = Some((1, y));
                        SetCapture(hwnd);
                        SetCursor(LoadCursorW(ptr::null_mut(), IDC_SIZENS));
                    } else if (y - s2).abs() <= 6 {
                        st.dragging_splitter = Some((2, y));
                        SetCapture(hwnd);
                        SetCursor(LoadCursorW(ptr::null_mut(), IDC_SIZENS));
                    }
                }
            });
            0
        }
        WM_MOUSEMOVE => {
            let y = ((lparam as u32 >> 16) & 0xFFFF) as i16 as i32;
            with_state(|st| {
                if let Some((split_id, last_y)) = st.dragging_splitter {
                    let delta = (y - last_y) as f64;
                    if delta.abs() >= 1.0 {
                        if split_id == 1 {
                            st.pane_layout.source_height =
                                (st.pane_layout.source_height + delta).clamp(68.0, 240.0);
                        } else if split_id == 2 {
                            st.pane_layout.web_height =
                                (st.pane_layout.web_height + delta).clamp(76.0, 400.0);
                        }
                        st.dragging_splitter = Some((split_id, y));
                        st.relayout_children();
                    }
                }
            });
            0
        }
        WM_LBUTTONUP => {
            with_state(|st| {
                if st.dragging_splitter.take().is_some() {
                    ReleaseCapture();
                    let _ = st.settings.save_pane_layout(st.pane_layout);
                }
            });
            0
        }
        WM_COMMAND => {
            let id = wparam & 0xFFFF;
            let code = ((wparam >> 16) & 0xFFFF) as u16;
            with_state(|st| handle_wm_command(st, id, code));
            0
        }
        WM_TRAYICON => {
            let mouse_msg = (lparam as u32) & 0xFFFF;
            if mouse_msg == WM_LBUTTONUP {
                with_state(|st| {
                    if IsWindowVisible(st.hwnd) != 0 {
                        st.hide_window();
                    } else {
                        st.show_and_focus();
                    }
                });
            } else if mouse_msg == WM_RBUTTONUP {
                show_tray_context_menu(hwnd);
            }
            0
        }
        WM_CLOSE => {
            with_state(|st| st.hide_window());
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn handle_wm_command(st: &mut Win32AppState, id: usize, code: u16) {
    match id {
        ID_BTN_TAB_TRANS if code == BN_CLICKED => st.switch_tab(TabMode::Translate),
        ID_BTN_TAB_CHAT if code == BN_CLICKED => st.switch_tab(TabMode::Chat),
        ID_BTN_TAB_SETTINGS if code == BN_CLICKED => st.switch_tab(TabMode::Settings),
        ID_BTN_OCR if code == BN_CLICKED => st.trigger_ocr(),
        ID_BTN_PIN if code == BN_CLICKED => st.set_pinned(!st.pinned),

        ID_EDIT_SOURCE if code == EN_CHANGE => {
            if !st.suppress_source_change {
                st.pending_auto_translate = Some(Instant::now() + AUTO_TRANSLATE_DEBOUNCE);
            }
        }
        ID_CMB_SOURCE_LANG if code == CBN_SELCHANGE => {
            let idx = SendMessageW(st.cmb_source_lang, CB_GETCURSEL, 0, 0) as usize;
            st.source_lang_idx = idx.min(SOURCE_LANGUAGES.len() - 1);
            st.start_translation();
        }
        ID_CMB_TARGET_LANG if code == CBN_SELCHANGE => {
            let idx = SendMessageW(st.cmb_target_lang, CB_GETCURSEL, 0, 0) as usize;
            st.target_lang_idx = idx.min(TARGET_LANGUAGES.len() - 1);
            st.start_translation();
        }
        ID_BTN_SWAP_LANG if code == BN_CLICKED => {
            let src_code = SOURCE_LANGUAGES[st.source_lang_idx].code;
            let tgt_code = TARGET_LANGUAGES[st.target_lang_idx].code;
            if src_code != "auto" && tgt_code != ADAPTIVE_TARGET {
                if let (Some(new_src), Some(new_tgt)) = (
                    SOURCE_LANGUAGES.iter().position(|l| l.code == tgt_code),
                    TARGET_LANGUAGES.iter().position(|l| l.code == src_code),
                ) {
                    st.source_lang_idx = new_src;
                    st.target_lang_idx = new_tgt;
                    SendMessageW(st.cmb_source_lang, CB_SETCURSEL, new_src, 0);
                    SendMessageW(st.cmb_target_lang, CB_SETCURSEL, new_tgt, 0);
                    st.start_translation();
                }
            }
        }
        ID_BTN_TRANS_NOW if code == BN_CLICKED => st.start_translation(),
        ID_BTN_CLEAR_TRANS if code == BN_CLICKED => {
            st.suppress_source_change = true;
            set_hwnd_text(st.edit_source, "");
            st.suppress_source_change = false;
            st.web_cancel.cancel_current();
            st.ai_cancel.cancel_current();
            set_hwnd_text(st.edit_web_result, "译文显示在这里");
            set_hwnd_text(st.edit_ai_result, "AI 译文显示在这里");
        }
        ID_BTN_COPY_WEB if code == BN_CLICKED => {
            copy_text_to_clipboard(st.hwnd, &st.web_result_text);
        }
        ID_BTN_COPY_AI if code == BN_CLICKED => {
            copy_text_to_clipboard(st.hwnd, &st.ai_stream_buf);
        }

        ID_CMB_CHAT_CONV if code == CBN_SELCHANGE => {
            let idx = SendMessageW(st.cmb_chat_conv, CB_GETCURSEL, 0, 0) as usize;
            if idx < st.conversations.len() {
                st.active_conv_idx = idx;
                st.current_messages = st.conversations[idx].messages.clone();
                st.render_chat_transcript();
            }
        }
        ID_BTN_CHAT_NEW if code == BN_CLICKED => {
            st.chat_cancel.cancel_current();
            st.chat_streaming = false;
            st.current_messages.clear();
            let _ = st.history.start_new_conversation();
            st.conversations = st.history.read_conversations().unwrap_or_default();
            st.active_conv_idx = st.conversations.len().saturating_sub(1);
            st.refresh_chat_controls();
        }
        ID_BTN_CHAT_COPY if code == BN_CLICKED => {
            if let Some(last_ai) = st
                .current_messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
            {
                copy_text_to_clipboard(st.hwnd, &last_ai.content);
            }
        }
        ID_BTN_CHAT_CLEAR if code == BN_CLICKED => {
            st.chat_cancel.cancel_current();
            st.chat_streaming = false;
            st.current_messages.clear();
            let _ = st.history.clear();
            st.conversations.clear();
            st.active_conv_idx = 0;
            st.refresh_chat_controls();
        }
        ID_BTN_CHAT_SEND if code == BN_CLICKED => st.send_chat_from_input(),
        ID_BTN_CHAT_STOP if code == BN_CLICKED => {
            st.chat_cancel.cancel_current();
            st.chat_streaming = false;
            st.render_chat_transcript();
        }

        ID_BTN_HYMT_CHECK if code == BN_CLICKED => {
            let tx = st.ui_tx.clone();
            st.rt.spawn_blocking(move || {
                let rep = hy_mt_manager::check_compatibility();
                let _ = tx.send(UiEvent::HyMtCompatReady(rep));
            });
        }
        ID_BTN_HYMT_RUN if code == BN_CLICKED => {
            let q_idx = (SendMessageW(st.cmb_hymt_quant, CB_GETCURSEL, 0, 0) as usize)
                .min(HyMtQuant::ALL.len() - 1);
            let quant = HyMtQuant::ALL[q_idx];
            let token = st.hymt_cancel.next();
            let tx_prog = st.ui_tx.clone();
            let tx_done = st.ui_tx.clone();
            st.rt.spawn(async move {
                let res = hy_mt_manager::download_and_start(quant, &token, |p| {
                    let _ = tx_prog.send(UiEvent::HyMtProgress(p));
                })
                .await;
                let _ = tx_done.send(UiEvent::HyMtReady(res));
            });
        }
        ID_BTN_HYMT_STOP if code == BN_CLICKED => {
            st.hymt_cancel.cancel_current();
            hy_mt_manager::stop_server();
            set_hwnd_text(st.lbl_hymt_status, "已停止内置 HY-MT1.5 本地服务");
        }
        ID_BTN_AI_DISCOVER if code == BN_CLICKED => {
            let b_idx = (SendMessageW(st.cmb_ai_backend, CB_GETCURSEL, 0, 0) as usize)
                .min(AiBackendKind::ALL.len() - 1);
            let backend = AiBackendKind::ALL[b_idx];
            let url = get_hwnd_text(st.edit_ai_url);
            let key = get_hwnd_text(st.edit_ai_key);
            let tx = st.ui_tx.clone();
            st.rt.spawn(async move {
                let opt_key = (!key.trim().is_empty()).then_some(key.as_str());
                let res = ttree_ai::discover_local_models(backend, &url, opt_key).await;
                let _ = tx.send(UiEvent::ModelsDiscovered(res));
            });
        }
        ID_BTN_CHAT_COPY_TRANS if code == BN_CLICKED => {
            let b_idx = SendMessageW(st.cmb_ai_backend, CB_GETCURSEL, 0, 0) as usize;
            SendMessageW(st.cmb_chat_backend, CB_SETCURSEL, b_idx, 0);
            set_hwnd_text(st.edit_chat_url, &get_hwnd_text(st.edit_ai_url));
            set_hwnd_text(st.edit_chat_key, &get_hwnd_text(st.edit_ai_key));
            set_hwnd_text(st.edit_chat_model, &get_hwnd_text(st.edit_ai_model));
        }
        ID_BTN_HK_RESET if code == BN_CLICKED => {
            let def = ShortcutConfig::default();
            set_hwnd_text(st.edit_hk_toggle, &def.toggle);
            set_hwnd_text(st.edit_hk_ocr, &def.ocr);
            set_hwnd_text(st.edit_hk_chat, &def.ai_dialog);
            set_hwnd_text(st.edit_hk_sel_trans, &def.selection_translate);
            set_hwnd_text(st.edit_hk_sel_chat, &def.selection_ai_dialog);
        }
        ID_BTN_SAVE_SETTINGS if code == BN_CLICKED => st.save_settings_from_ui(),

        ID_TRAY_TRANSLATE => {
            st.switch_tab(TabMode::Translate);
            st.show_and_focus();
        }
        ID_TRAY_OCR => st.trigger_ocr(),
        ID_TRAY_CHAT => {
            st.switch_tab(TabMode::Chat);
            st.show_and_focus();
        }
        ID_TRAY_SETTINGS => {
            st.switch_tab(TabMode::Settings);
            st.show_and_focus();
        }
        ID_TRAY_QUIT => {
            DestroyWindow(st.hwnd);
        }
        _ => {}
    }
}
