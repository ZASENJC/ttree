//! `TTREE` Windows 100% 纯 Rust 原生 Win32 + DWM 桌面端实现。
//!
//! 架构与 macOS AppKit 端完全对称：
//! - 复用 `ttree-core`、`ttree-storage`、`ttree-translate`、`ttree-ai`、`ttree-ocr`、`ttree-platform`；
//! - 原生 Win32 控件树 + DWM 深色模式 & Mica/Acrylic 云母材质 + 分层窗口透明度 (`SetLayeredWindowAttributes`)；
//! - 系统托盘 (`Shell_NotifyIconW`) + 25ms `SetTimer` 驱动 `global-hotkey` 与 Tokio `mpsc` 流式 UI 事件；
//! - 完整支持四段式翻译面板（含可拖拽高度分隔条）、AI 多轮流式对话、内置 `HY-MT1.5-1.8B` 一键下载托管、
//!   以及增强版双通道多语言 Windows 截图 OCR。

#![cfg(target_os = "windows")]
#![allow(non_snake_case, clippy::too_many_arguments)]

mod dialogs;
mod shortcut_recorder;
mod ui;

use crate::design::{BoxRect, TranslationLayout, SIDEBAR, TOOLBAR};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use ui::*;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use ttree_ai::hy_mt_manager::{self, HyMtCompatibilityReport, HyMtDownloadProgress};
use ttree_core::{
    normalize_ocr_newlines, recent_chat_context, resolve_language_pair, AiBackendKind,
    AiTranslationConfig, AppearanceConfig, CancelSlot, ChatAiConfig, ChatMessage, Conversation,
    HyMtQuant, PaneLayoutConfig, ShortcutConfig, ADAPTIVE_TARGET, PANEL_OPACITY_MAX,
    PANEL_OPACITY_MIN, SOURCE_LANGUAGES, TARGET_LANGUAGES, WINDOW_MAX_HEIGHT, WINDOW_MAX_WIDTH,
    WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
use ttree_ocr::OcrOutcome;
use ttree_platform::autostart;
use ttree_platform::hotkey::{
    parse_shortcut_config, register_shortcuts, ParsedShortcuts, ShortcutAction,
};
use ttree_platform::selection::capture_selected_text;
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
const WS_THICKFRAME: u32 = 0x00040000;
const WS_CLIPCHILDREN: u32 = 0x02000000;
const WS_CHILD: u32 = 0x40000000;
const WS_VSCROLL: u32 = 0x00200000;
const WS_BORDER: u32 = 0x00800000;

const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_APPWINDOW: u32 = 0x00040000;
const WS_EX_LAYERED: u32 = 0x00080000;
const GWL_EXSTYLE: i32 = -20;

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

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const SW_RESTORE: i32 = 9;

const HWND_TOPMOST: HWND = -1isize as HWND;
const HWND_NOTOPMOST: HWND = -2isize as HWND;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_FRAMECHANGED: u32 = 0x0020;

const LWA_ALPHA: u32 = 0x00000002;
const TRANSPARENT: i32 = 1;
const OPAQUE: i32 = 2;

const NIM_ADD: u32 = 0x00000000;
const NIM_MODIFY: u32 = 0x00000001;
const NIM_DELETE: u32 = 0x00000002;
const NIF_MESSAGE: u32 = 0x00000001;
const NIF_ICON: u32 = 0x00000002;
const NIF_TIP: u32 = 0x00000004;

const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_BOTTOMALIGN: u32 = 0x0020;
const MF_STRING: u32 = 0x0000;
const MF_SEPARATOR: u32 = 0x0800;

const IDI_TTREE: *const u16 = 1 as *const u16;
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
const ID_BTN_UPDATE: usize = 428;
const ID_CHK_AUTOSTART: usize = 425;
const ID_BTN_SAVE_SETTINGS: usize = 426;
const ID_BTN_HYMT_DELETE: usize = 427;
const ID_CHK_NORMAL_WINDOW: usize = 429;

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
    HyMtProgress {
        seq: u64,
        progress: HyMtDownloadProgress,
    },
    HyMtReady {
        seq: u64,
        result: Result<String, String>,
    },
    HyMtRemoved {
        seq: u64,
        result: Result<(), String>,
    },
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
    fn EnableWindow(hWnd: HWND, enable: BOOL) -> BOOL;
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
    updater: ttree_updater::Updater,
    update_view: ttree_updater::View,
    btn_update: HWND,
    lbl_update: HWND,
    rt: TokioHandle,
    ui_tx: UnboundedSender<UiEvent>,
    ui_rx: UnboundedReceiver<UiEvent>,
    settings: Arc<SettingsStore>,
    history: Arc<HistoryStore>,
    hotkey_mgr: GlobalHotKeyManager,
    registered_hotkeys: Vec<HotKey>,
    recording_shortcut: bool,
    parsed_shortcuts: ParsedShortcuts,

    hwnd: HWND,
    font_regular: HFONT,
    font_bold: HFONT,
    brush_bg: HBRUSH,
    brush_surface: HBRUSH,
    brush_field: HBRUSH,
    dpi: u32,
    settings_section: usize,
    settings_return_size: Option<(i32, i32)>,
    settings_view: HWND,
    settings_nav: [HWND; 4],
    btn_done: HWND,
    btn_close: HWND,
    btn_history: HWND,
    history_visible: bool,
    chat_bubbles: Vec<HWND>,
    settings_labels: Vec<HWND>,

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
    hymt_downloading: bool,
    hymt_removing: bool,
    hymt_seq: u64,
    ignored_ai_seq: Option<u64>,
    hymt_cancel_requested: bool,

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
    btn_hymt_delete: HWND,
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
    chk_normal_window: HWND,
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
    let show_on_launch = crate::startup::should_show(std::env::args_os().skip(1));
    let _instance = match crate::startup::acquire(show_on_launch) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            crate::startup::report_error(&error);
            return;
        }
    };
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
        let class_name = wide_null(crate::startup::WINDOW_CLASS);
        let colors = palette();
        let bg_brush = CreateSolidBrush(colors.bg);
        let surface_brush = CreateSolidBrush(colors.surface);
        let field_brush = CreateSolidBrush(colors.field);

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: main_wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: LoadIconW(GetModuleHandleW(ptr::null()), IDI_TTREE),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: bg_brush,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: LoadIconW(GetModuleHandleW(ptr::null()), IDI_TTREE),
        };
        RegisterClassExW(&wc);

        let dpi = GetDpiForSystem().max(96);
        let font_regular = font(13, 400, dpi);
        let font_bold = font(13, 600, dpi);
        configure(dpi, font_regular, font_bold);

        let viewport_class = wide_null("TTreeNativeViewport");
        let viewport_wc = WNDCLASSEXW {
            lpfnWndProc: viewport_proc,
            hbrBackground: ptr::null_mut(),
            lpszClassName: viewport_class.as_ptr(),
            ..wc
        };
        RegisterClassExW(&viewport_wc);

        let title = wide_null("TTREE");
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_COMPOSITED
                | if settings.appearance().normal_window_mode {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_THICKFRAME | WS_CLIPCHILDREN,
            260,
            180,
            scale(win_w),
            scale(win_h),
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            panic!("创建 Windows 主窗口失败");
        }

        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | 4 | 0x20,
        );
        // Match the system palette, retaining DWM corners and backdrop where available.
        let dark_mode: u32 = colors.dark() as u32;
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
            recording_shortcut: false,
            parsed_shortcuts: ParsedShortcuts::default(),
            hwnd,
            font_regular,
            font_bold,
            brush_bg: bg_brush,
            brush_surface: surface_brush,
            brush_field: field_brush,
            dpi,
            settings_section: ID_SETTINGS_TRANS,
            settings_return_size: None,
            settings_view: ptr::null_mut(),
            settings_nav: [ptr::null_mut(); 4],
            btn_done: ptr::null_mut(),
            btn_close: ptr::null_mut(),
            btn_history: ptr::null_mut(),
            history_visible: true,
            chat_bubbles: Vec::new(),
            settings_labels: Vec::new(),
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
            hymt_downloading: false,
            hymt_removing: false,
            hymt_seq: 0,
            ignored_ai_seq: None,
            hymt_cancel_requested: false,
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
            btn_hymt_delete: ptr::null_mut(),
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
            chk_normal_window: ptr::null_mut(),
            updater: Default::default(),
            update_view: Default::default(),
            btn_update: ptr::null_mut(),
            lbl_update: ptr::null_mut(),
            btn_save_settings: ptr::null_mut(),
            lbl_settings_status: ptr::null_mut(),
        };

        app_state.build_child_controls(hinstance);
        SendMessageW(hwnd, 0x0127, (1 << 16) | 1, 0); // UIS_SET | UISF_HIDEFOCUS; keyboard navigation restores it.
        let sc = app_state.settings.shortcuts();
        let shortcut_result = app_state.apply_shortcuts(&sc);
        app_state.populate_settings_fields();
        if let Err(error) = shortcut_result {
            set_hwnd_text(app_state.lbl_settings_status, &error);
        }
        app_state.refresh_chat_controls();
        app_state.relayout_children();

        install_tray_icon(hwnd);
        SetTimer(hwnd, TIMER_ID_POLL, 25, ptr::null());

        WIN_CTX.with(|cell| {
            *cell.borrow_mut() = Some(app_state);
        });
        if show_on_launch {
            with_state(|st| st.show_and_focus());
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            with_state(|state| {
                state.set_shortcut_recording(
                    shortcut_recorder::is_field(GetFocus()) && IsWindowVisible(GetFocus()) != 0,
                );
            });
            // Let text-input subclasses handle Return before dialog navigation consumes it.
            if (msg.message == WM_KEYDOWN || msg.message == WM_CHAR)
                && (msg.wParam == 0x0d || msg.wParam == 0x1b)
                || IsDialogMessageW(hwnd, &mut msg) == 0
            {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        cleanup();
        remove_tray_icon(hwnd);
        hy_mt_manager::stop_server();
        WIN_CTX.with(|cell| {
            if let Some(st) = cell.borrow_mut().take() {
                DeleteObject(st.font_regular);
                DeleteObject(st.font_bold);
                DeleteObject(st.brush_bg);
                DeleteObject(st.brush_surface);
                DeleteObject(st.brush_field);
            }
        });
    }
}

unsafe fn install_tray_icon(hwnd: HWND) {
    set_tray_icon(hwnd, NIM_ADD);
}

unsafe fn set_tray_icon(hwnd: HWND, operation: u32) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    // System/taskbar appearance may differ from the application's theme.
    let glyph_id = if taskbar_uses_light_theme() { 2 } else { 3 };
    nid.hIcon = LoadIconW(GetModuleHandleW(ptr::null()), glyph_id as *const u16);
    assert!(!nid.hIcon.is_null(), "bundled tray glyph is missing");
    let tip = wide_null("TTREE AI 翻译与 OCR");
    let copy_len = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);
    Shell_NotifyIconW(operation, &nid);
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
        let style = if class == "BUTTON" && style != BS_AUTOCHECKBOX {
            BS_OWNERDRAW
        } else if class == "COMBOBOX" {
            style | CBS_OWNERDRAWFIXED
        } else if class == "EDIT" {
            style & !WS_BORDER
        } else {
            style
        };
        let cls = wide_null(class);
        let txt = wide_null(title);
        let ctrl = CreateWindowExW(
            0,
            cls.as_ptr(),
            txt.as_ptr(),
            WS_CHILD | WS_CLIPSIBLINGS | style | if class == "STATIC" { 0 } else { WS_TABSTOP },
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
        init_control(ctrl, class, id);
        ctrl
    }

    unsafe fn build_child_controls(&mut self, hinst: HINSTANCE) {
        // 1. 顶部导航栏
        self.btn_tab_trans = self.create_ctrl(
            hinst,
            "BUTTON",
            "翻译",
            BS_PUSHBUTTON,
            ID_BTN_TAB_TRANS,
            true,
        );
        self.btn_tab_chat =
            self.create_ctrl(hinst, "BUTTON", "AI", BS_PUSHBUTTON, ID_BTN_TAB_CHAT, false);
        self.btn_ocr = self.create_ctrl(
            hinst,
            "BUTTON",
            "截图 OCR",
            BS_PUSHBUTTON,
            ID_BTN_OCR,
            false,
        );
        self.btn_pin = self.create_ctrl(
            hinst,
            "BUTTON",
            "固定窗口",
            BS_PUSHBUTTON,
            ID_BTN_PIN,
            false,
        );
        self.btn_tab_settings = self.create_ctrl(
            hinst,
            "BUTTON",
            "设置",
            BS_PUSHBUTTON,
            ID_BTN_TAB_SETTINGS,
            false,
        );

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

        self.btn_trans_now = self.create_ctrl(
            hinst,
            "BUTTON",
            "翻译",
            BS_PUSHBUTTON,
            ID_BTN_TRANS_NOW,
            true,
        );
        self.btn_clear_trans = self.create_ctrl(
            hinst,
            "BUTTON",
            "清空",
            BS_PUSHBUTTON,
            ID_BTN_CLEAR_TRANS,
            false,
        );

        self.lbl_web_header =
            self.create_ctrl(hinst, "STATIC", "翻译  ·  GOOGLE / BING", 0, 0, true);
        self.btn_copy_web = self.create_ctrl(
            hinst,
            "BUTTON",
            "复制",
            BS_PUSHBUTTON,
            ID_BTN_COPY_WEB,
            false,
        );
        self.edit_web_result = self.create_ctrl(
            hinst,
            "EDIT",
            "译文显示在这里",
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            ID_EDIT_WEB_RESULT,
            false,
        );

        self.lbl_ai_header = self.create_ctrl(hinst, "STATIC", "AI", 0, 0, true);
        self.btn_copy_ai = self.create_ctrl(
            hinst,
            "BUTTON",
            "复制",
            BS_PUSHBUTTON,
            ID_BTN_COPY_AI,
            false,
        );
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
        self.btn_chat_new = self.create_ctrl(
            hinst,
            "BUTTON",
            "+ 新对话",
            BS_PUSHBUTTON,
            ID_BTN_CHAT_NEW,
            false,
        );
        self.btn_chat_copy = self.create_ctrl(
            hinst,
            "BUTTON",
            "复制回复",
            BS_PUSHBUTTON,
            ID_BTN_CHAT_COPY,
            false,
        );
        self.btn_chat_clear = self.create_ctrl(
            hinst,
            "BUTTON",
            "清空历史",
            BS_PUSHBUTTON,
            ID_BTN_CHAT_CLEAR,
            false,
        );
        self.edit_chat_history = self.create_ctrl(
            hinst,
            "TTreeNativeViewport",
            "",
            WS_VSCROLL | WS_CLIPCHILDREN,
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
        self.btn_chat_send = self.create_ctrl(
            hinst,
            "BUTTON",
            "发送",
            BS_PUSHBUTTON,
            ID_BTN_CHAT_SEND,
            true,
        );
        self.btn_chat_stop = self.create_ctrl(
            hinst,
            "BUTTON",
            "停止",
            BS_PUSHBUTTON,
            ID_BTN_CHAT_STOP,
            false,
        );

        // 4. 设置面板控件
        self.chk_ai_enabled = self.create_ctrl(
            hinst,
            "BUTTON",
            "启用 AI 翻译",
            BS_AUTOCHECKBOX,
            ID_CHK_AI_ENABLED,
            true,
        );
        self.lbl_ai_backend = self.create_ctrl(hinst, "STATIC", "接入方式", 0, 0, false);
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

        self.lbl_hymt_quant = self.create_ctrl(hinst, "STATIC", "模型精度", 0, 0, false);
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
        self.btn_hymt_check = self.create_ctrl(
            hinst,
            "BUTTON",
            "硬件检测",
            BS_PUSHBUTTON,
            ID_BTN_HYMT_CHECK,
            false,
        );
        self.btn_hymt_run = self.create_ctrl(
            hinst,
            "BUTTON",
            "下载并运行",
            BS_PUSHBUTTON,
            ID_BTN_HYMT_RUN,
            true,
        );
        self.btn_hymt_stop = self.create_ctrl(
            hinst,
            "BUTTON",
            "停止服务",
            BS_PUSHBUTTON,
            ID_BTN_HYMT_STOP,
            false,
        );
        self.btn_hymt_delete = self.create_ctrl(
            hinst,
            "BUTTON",
            "删除服务",
            BS_PUSHBUTTON,
            ID_BTN_HYMT_DELETE,
            false,
        );
        self.lbl_hymt_status = self.create_ctrl(
            hinst,
            "STATIC",
            "内置 HY-MT1.5-1.8B 就绪状态: 待检测",
            0,
            0,
            false,
        );

        self.lbl_ai_url = self.create_ctrl(hinst, "STATIC", "接口地址", 0, 0, false);
        self.edit_ai_url = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL,
            ID_EDIT_AI_URL,
            false,
        );
        self.lbl_ai_key = self.create_ctrl(hinst, "STATIC", "API Key", 0, 0, false);
        self.edit_ai_key = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL | ES_PASSWORD,
            ID_EDIT_AI_KEY,
            false,
        );
        self.lbl_ai_model = self.create_ctrl(hinst, "STATIC", "模型名称", 0, 0, false);
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
        self.lbl_ai_prompt = self.create_ctrl(hinst, "STATIC", "翻译提示词", 0, 0, false);
        self.edit_ai_prompt = self.create_ctrl(
            hinst,
            "EDIT",
            "",
            WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN,
            ID_EDIT_AI_PROMPT,
            false,
        );

        self.lbl_chat_cfg = self.create_ctrl(hinst, "STATIC", "AI 对话", 0, 0, true);
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

        self.lbl_hk_header = self.create_ctrl(hinst, "STATIC", "快捷键", 0, 0, true);
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

        self.lbl_opacity = self.create_ctrl(hinst, "STATIC", "窗口不透明度", 0, 0, false);
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
        self.chk_normal_window = self.create_ctrl(
            hinst,
            "BUTTON",
            "普通窗口模式",
            BS_AUTOCHECKBOX,
            ID_CHK_NORMAL_WINDOW,
            false,
        );
        self.btn_update = self.create_ctrl(
            hinst,
            "BUTTON",
            "检查更新",
            BS_PUSHBUTTON,
            ID_BTN_UPDATE,
            false,
        );
        self.lbl_update = self.create_ctrl(
            hinst,
            "STATIC",
            &ttree_updater::View::default().status,
            0,
            0,
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
        small_label(self.lbl_web_header);
        small_label(self.lbl_ai_header);
        self.settings_view = self.create_ctrl(
            hinst,
            "TTreeNativeViewport",
            "",
            WS_VSCROLL | WS_CLIPCHILDREN,
            ID_SETTINGS_VIEW,
            false,
        );
        for (i, (id, title)) in [
            (ID_SETTINGS_TRANS, "AI 翻译"),
            (ID_SETTINGS_CHAT, "AI 对话"),
            (ID_SETTINGS_KEYS, "快捷键"),
            (ID_SETTINGS_GENERAL, "通用"),
        ]
        .into_iter()
        .enumerate()
        {
            self.settings_nav[i] =
                self.create_ctrl(hinst, "BUTTON", title, BS_PUSHBUTTON, id, false);
        }
        self.btn_done = self.create_ctrl(
            hinst,
            "BUTTON",
            "完成",
            BS_PUSHBUTTON,
            ID_SETTINGS_DONE,
            false,
        );
        self.btn_close =
            self.create_ctrl(hinst, "BUTTON", "隐藏窗口", BS_PUSHBUTTON, ID_CLOSE, false);
        self.btn_history = self.create_ctrl(
            hinst,
            "BUTTON",
            "历史对话",
            BS_PUSHBUTTON,
            ID_HISTORY_TOGGLE,
            false,
        );
        for c in self.settings_controls() {
            SetParent(c, self.settings_view);
        }
        for title in [
            "接入方式",
            "接口地址",
            "API Key",
            "模型名称",
            "呼出翻译",
            "截图 OCR",
            "AI 对话",
            "划词翻译",
            "划词对话",
        ] {
            let label = self.create_ctrl(hinst, "STATIC", title, 0, 0, false);
            SetParent(label, self.settings_view);
            self.settings_labels.push(label);
        }
        cue(self.edit_ai_url, "https://api.openai.com/v1");
        cue(self.edit_chat_url, "https://api.openai.com/v1");
        cue(self.edit_ai_key, "输入 API Key");
        cue(self.edit_chat_key, "输入 API Key");
        for c in self
            .settings_controls()
            .into_iter()
            .chain(self.settings_labels.iter().copied())
        {
            control_background(c, 3);
        }
        for c in [
            self.edit_ai_url,
            self.edit_ai_key,
            self.edit_ai_model,
            self.edit_ai_prompt,
            self.edit_chat_url,
            self.edit_chat_key,
            self.edit_chat_model,
            self.edit_hk_toggle,
            self.edit_hk_ocr,
            self.edit_hk_chat,
            self.edit_hk_sel_trans,
            self.edit_hk_sel_chat,
        ] {
            control_background(c, 2);
        }
        install_tooltips(
            self.hwnd,
            &[
                (self.btn_ocr, "截图翻译"),
                (self.btn_pin, "固定 / 取消固定窗口"),
                (self.btn_tab_settings, "偏好设置"),
                (self.btn_close, "隐藏窗口"),
                (self.btn_chat_new, "新建对话"),
                (self.btn_history, "展开 / 收起历史对话"),
            ],
        );
        cue(self.edit_ai_model, "模型名称");
        cue(self.edit_chat_model, "模型名称");
    }

    fn settings_controls(&self) -> Vec<HWND> {
        vec![
            self.chk_ai_enabled,
            self.lbl_ai_backend,
            self.cmb_ai_backend,
            self.lbl_hymt_quant,
            self.cmb_hymt_quant,
            self.btn_hymt_check,
            self.btn_hymt_run,
            self.btn_hymt_stop,
            self.btn_hymt_delete,
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
            self.edit_chat_url,
            self.edit_chat_key,
            self.edit_chat_model,
            self.btn_chat_copy_trans,
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
            self.chk_normal_window,
            self.btn_update,
            self.lbl_update,
        ]
    }

    unsafe fn client_size(&self) -> (i32, i32) {
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut r);
        (unscale(r.right).max(1), unscale(r.bottom).max(1))
    }

    unsafe fn translation_layout(&self) -> TranslationLayout {
        let (w, h) = self.client_size();
        TranslationLayout::new(
            w,
            h,
            self.pane_layout.source_height as i32,
            self.pane_layout.lang_height as i32,
            self.pane_layout.web_height as i32,
            self.settings.ai_translation_for_request().enabled,
        )
    }

    unsafe fn layout_source_input(&self, source: BoxRect, has_text: bool) {
        // Commit only the final width: widening then narrowing a populated edit
        // reformats its contents twice, even when the window layout is unchanged.
        place(
            self.edit_source,
            14,
            source.y + 6,
            source.w - if has_text { 76 } else { 28 },
            (source.h - 12).max(12),
        );
        if has_text {
            place(self.btn_clear_trans, source.w - 58, source.y + 4, 44, 24);
        }
        visible(self.btn_clear_trans, has_text);
    }

    unsafe fn refresh_source_input(&self) {
        if self.active_tab == TabMode::Translate {
            let has_text = GetWindowTextLengthW(self.edit_source) > 0;
            if shown(self.btn_clear_trans) != has_text {
                let source = self.translation_layout().source;
                self.layout_source_input(source, has_text);
                let area = RECT {
                    left: scale(source.x),
                    top: scale(source.y),
                    right: scale(source.x + source.w),
                    bottom: scale(source.y + source.h),
                };
                InvalidateRect(self.hwnd, &area, 0);
                InvalidateRect(self.edit_source, ptr::null(), 0);
            }
        }
        // Text can add/remove wrapped lines without changing the edit's bounds.
        edit_geometry(self.edit_source);
    }

    unsafe fn relayout_children(&self) {
        let _frame = LayoutFrame::new(self.hwnd, self.hwnd);
        let (w, h) = self.client_size();
        let p = palette();
        selection(self.active_tab, self.settings_section, self.pinned);
        let mut bg = Scene::default();
        let settings = self.active_tab == TabMode::Settings;
        let chat = self.active_tab == TabMode::Chat;
        let show = |ctrl, show| {
            visible(ctrl, show);
        };
        // Same 128px segmented navigation and quiet icon toolbar as macOS.
        bg.surface(BoxRect::new(16, 10, 128, 28), p.surface, 8);
        place(self.btn_tab_trans, 19, 12, 59, 24);
        place(self.btn_tab_chat, 82, 12, 59, 24);
        show(self.btn_tab_trans, !settings);
        show(self.btn_tab_chat, !settings);
        place(self.btn_ocr, w - 152, 8, 32, 32);
        show(self.btn_ocr, !settings && !chat);
        place(self.btn_pin, w - 116, 8, 32, 32);
        show(self.btn_pin, !settings);
        place(self.btn_tab_settings, w - 80, 8, 32, 32);
        show(self.btn_tab_settings, !settings);
        place(self.btn_chat_new, w - 188, 8, 32, 32);
        show(self.btn_chat_new, chat);
        place(self.btn_history, w - 152, 8, 32, 32);
        show(self.btn_history, chat);
        place(self.btn_close, w - 44, 8, 32, 32);
        show(self.btn_close, true);
        place(self.btn_done, w - 102, 11, 48, 26);
        show(self.btn_done, settings);
        if settings {
            bg.surface(BoxRect::new(0, 0, 150, TOOLBAR), p.bg, 0);
            bg.label(BoxRect::new(18, 12, 100, 24), "偏好设置", false, false);
        }

        for c in [
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
        ] {
            show(c, self.active_tab == TabMode::Translate);
        }
        for c in [
            self.cmb_chat_conv,
            self.btn_chat_copy,
            self.btn_chat_clear,
            self.edit_chat_history,
            self.edit_chat_input,
            self.btn_chat_send,
            self.btn_chat_stop,
        ] {
            show(c, chat);
        }
        if !settings {
            for c in self
                .settings_controls()
                .into_iter()
                .chain(self.settings_labels.iter().copied())
            {
                show(c, false);
            }
        }
        for c in self.settings_nav {
            show(c, settings);
        }
        show(self.settings_view, settings);
        show(self.btn_save_settings, settings);
        show(self.lbl_settings_status, settings);

        match self.active_tab {
            TabMode::Translate => {
                let l = self.translation_layout();
                bg.surface(l.source, p.surface, 0);
                bg.surface(l.languages, p.surface, 0);
                self.layout_source_input(l.source, GetWindowTextLengthW(self.edit_source) > 0);
                show(self.edit_source, true);
                let ly = l.languages.y + (l.languages.h - 30) / 2;
                let left = if w < 430 { 100 } else { 114 };
                let target = (w - left - 94).clamp(120, 152);
                place_combo(self.cmb_source_lang, 12, ly, left, 30, 220);
                show(self.cmb_source_lang, true);
                place(self.btn_swap_lang, 12 + left, ly + 2, 24, 26);
                show(self.btn_swap_lang, true);
                place_combo(self.cmb_target_lang, 38 + left, ly, target, 30, 220);
                show(self.cmb_target_lang, true);
                place(self.btn_trans_now, w - 60, ly + 2, 48, 26);
                show(self.btn_trans_now, true);
                bg.grip(l.languages.y, w, p.line);
                bg.grip(l.web.y, w, p.line);
                place(self.lbl_web_header, 16, l.web.y + 7, w - 90, 17);
                show(self.lbl_web_header, true);
                place(self.btn_copy_web, w - 58, l.web.y + 4, 46, 22);
                show(self.btn_copy_web, !self.web_result_text.is_empty());
                place(
                    self.edit_web_result,
                    14,
                    l.web.y + 25,
                    w - 28,
                    (l.web.h - 31).max(12),
                );
                show(self.edit_web_result, true);
                for c in [self.lbl_ai_header, self.btn_copy_ai, self.edit_ai_result] {
                    show(c, l.ai.h > 0);
                }
                if l.ai.h > 0 {
                    bg.grip(l.ai.y, w, p.line);
                    place(self.lbl_ai_header, 16, l.ai.y + 7, w - 90, 17);
                    show(self.lbl_ai_header, true);
                    place(self.btn_copy_ai, w - 58, l.ai.y + 4, 46, 22);
                    show(self.btn_copy_ai, !self.ai_stream_buf.is_empty());
                    place(
                        self.edit_ai_result,
                        14,
                        l.ai.y + 25,
                        w - 28,
                        (l.ai.h - 31).max(12),
                    );
                    show(self.edit_ai_result, true);
                }
            }
            TabMode::Chat => {
                let hist_h = if self.history_visible { 38 } else { 0 };
                show(self.cmb_chat_conv, self.history_visible);
                show(self.btn_chat_clear, self.history_visible);
                if self.history_visible {
                    bg.surface(BoxRect::new(0, TOOLBAR, w, hist_h), p.surface, 0);
                    bg.label(
                        BoxRect::new(14, TOOLBAR + 10, 58, 18),
                        "历史对话",
                        false,
                        true,
                    );
                    place_combo(self.cmb_chat_conv, 74, TOOLBAR + 4, w - 218, 30, 220);
                    show(self.cmb_chat_conv, true);
                    place(self.btn_chat_clear, w - 66, TOOLBAR + 6, 54, 24);
                    show(self.btn_chat_clear, true);
                    bg.divider(TOOLBAR + hist_h, w, p.line);
                }
                let input = self.pane_layout.chat_input_height as i32;
                let input = input
                    .clamp(68, 220)
                    .min((h - TOOLBAR - hist_h - 80).max(68));
                let top = h - input;
                place(
                    self.edit_chat_history,
                    0,
                    TOOLBAR + hist_h,
                    w,
                    (top - TOOLBAR - hist_h).max(20),
                );
                show(self.edit_chat_history, true);
                bg.surface(BoxRect::new(0, top, w, input), p.surface, 0);
                bg.grip(top, w, p.line);
                place(self.edit_chat_input, 14, top + 8, w - 100, input - 16);
                show(self.edit_chat_input, true);
                place(self.btn_chat_send, w - 74, top + (input - 28) / 2, 62, 28);
                show(self.btn_chat_send, !self.chat_streaming);
                place(self.btn_chat_stop, w - 74, top + (input - 28) / 2, 62, 28);
                show(self.btn_chat_stop, self.chat_streaming);
                place(self.btn_chat_copy, w - 138, TOOLBAR + 6, 68, 24);
                show(
                    self.btn_chat_copy,
                    self.history_visible && !self.current_messages.is_empty(),
                );
            }
            TabMode::Settings => {
                bg.surface(BoxRect::new(0, TOOLBAR, SIDEBAR, h - TOOLBAR), p.surface, 0);
                bg.divider(TOOLBAR, w, p.line);
                bg.surface(BoxRect::new(SIDEBAR, TOOLBAR, 1, h - TOOLBAR), p.line, 0);
                for (i, c) in self.settings_nav.iter().enumerate() {
                    place(*c, 8, TOOLBAR + 18 + i as i32 * 42, SIDEBAR - 16, 34);
                }
                place(
                    self.settings_view,
                    SIDEBAR + 1,
                    TOOLBAR + 1,
                    w - SIDEBAR - 1,
                    (h - TOOLBAR - 90).max(20),
                );
                bg.surface(BoxRect::new(SIDEBAR, h - 64, w - SIDEBAR, 1), p.line, 0);
                place(self.btn_save_settings, w - 126, h - 48, 104, 32);
                place(
                    self.lbl_settings_status,
                    SIDEBAR + 22,
                    h - 88,
                    w - SIDEBAR - 44,
                    20,
                );
                bg.label(
                    BoxRect::new(SIDEBAR + 22, h - 43, (w - SIDEBAR - 166).max(0), 22),
                    "保存后立即生效",
                    true,
                    true,
                );
                self.layout_settings();
            }
        }
        scene(self.hwnd, bg);
        InvalidateRect(self.hwnd, ptr::null(), 0);
        for ctrl in [
            self.btn_tab_trans,
            self.btn_tab_chat,
            self.btn_pin,
            self.btn_tab_settings,
        ]
        .into_iter()
        .chain(self.settings_nav)
        {
            InvalidateRect(ctrl, ptr::null(), 0);
        }
    }

    unsafe fn layout_settings(&self) {
        let _frame = LayoutFrame::new(self.hwnd, self.settings_view);
        self.layout_settings_pass(0);
    }

    unsafe fn layout_settings_pass(&self, pass: u8) {
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(self.settings_view, &mut r);
        let w = unscale(r.right);
        let h = unscale(r.bottom);
        let p = palette();
        let mut bg = Scene::default();
        let section = self.settings_section;
        let built_in = SendMessageW(self.cmb_ai_backend, CB_GETCURSEL, 0, 0) == 0;
        let desired: Vec<HWND> = match section {
            ID_SETTINGS_TRANS => {
                let mut controls = vec![
                    self.lbl_ai_backend,
                    self.cmb_ai_backend,
                    self.chk_ai_enabled,
                ];
                if built_in {
                    controls.extend([
                        self.lbl_hymt_quant,
                        self.cmb_hymt_quant,
                        self.btn_hymt_check,
                        self.btn_hymt_run,
                        self.btn_hymt_stop,
                        self.btn_hymt_delete,
                        self.lbl_hymt_status,
                    ]);
                } else {
                    controls.extend([
                        self.lbl_ai_url,
                        self.edit_ai_url,
                        self.lbl_ai_key,
                        self.edit_ai_key,
                        self.lbl_ai_model,
                        self.edit_ai_model,
                        self.lbl_ai_prompt,
                        self.edit_ai_prompt,
                    ]);
                    let b = SendMessageW(self.cmb_ai_backend, CB_GETCURSEL, 0, 0);
                    if b == 2 || b == 3 {
                        controls.push(self.btn_ai_discover);
                    }
                }
                controls
            }
            ID_SETTINGS_CHAT => [
                vec![
                    self.btn_chat_copy_trans,
                    self.cmb_chat_backend,
                    self.edit_chat_url,
                    self.edit_chat_key,
                    self.edit_chat_model,
                ],
                self.settings_labels[..4].to_vec(),
            ]
            .concat(),
            ID_SETTINGS_KEYS => [
                vec![
                    self.edit_hk_toggle,
                    self.edit_hk_ocr,
                    self.edit_hk_chat,
                    self.edit_hk_sel_trans,
                    self.edit_hk_sel_chat,
                    self.btn_hk_reset,
                ],
                self.settings_labels[4..].to_vec(),
            ]
            .concat(),
            _ => vec![
                self.lbl_opacity,
                self.cmb_opacity,
                self.chk_ocr_newline,
                self.chk_autostart,
                self.chk_normal_window,
                self.btn_update,
                self.lbl_update,
            ],
        };
        for c in self
            .settings_controls()
            .into_iter()
            .chain(self.settings_labels.iter().copied())
        {
            if !desired.contains(&c) {
                visible(c, false);
            }
        }
        let x = 18;
        let fw = (w - 36).max(160);
        let inset = 16;
        let wide = fw >= 350;
        let row_h = if wide { 46 } else { 72 };
        let service_h = if wide { 116 } else { 142 };
        let chat_service_h = if wide { 82 } else { 104 };
        let content = match section {
            ID_SETTINGS_TRANS => {
                if built_in {
                    82 + service_h + 16 + if wide { 256 } else { 282 } + 16
                } else {
                    82 + service_h + 16 + (row_h * 3 + 32) + 16 + 152 + 16
                }
            }
            ID_SETTINGS_CHAT => 82 + chat_service_h + 16 + row_h * 3 + 32 + 16,
            ID_SETTINGS_KEYS => 82 + row_h * 5 + 32 + 90,
            _ => {
                if wide {
                    568
                } else {
                    608
                }
            }
        };
        let offset = scroll_range(self.settings_view, content, h);
        let mut updated: RECT = std::mem::zeroed();
        GetClientRect(self.settings_view, &mut updated);
        // Showing/hiding the native scrollbar changes the client width. Resolve
        // the responsive layout with that width before publishing any child moves.
        if unscale(updated.right) != w && pass < 2 {
            self.layout_settings_pass(pass + 1);
            return;
        }
        let (title, subtitle) = match section {
            ID_SETTINGS_TRANS => ("AI 翻译", "选择翻译服务，调整模型与提示词"),
            ID_SETTINGS_CHAT => ("AI 对话", "独立配置你的对话服务"),
            ID_SETTINGS_KEYS => ("快捷键", "快速呼出窗口、截图与划词操作"),
            _ => ("通用", "让 TTREE 适合你的使用习惯"),
        };
        bg.heading(BoxRect::new(x, 20 - offset, fw, 28), title);
        bg.label(BoxRect::new(x, 52 - offset, fw, 20), subtitle, true, true);
        let show = |ctrl| {
            visible(ctrl, true);
        };
        let form_row = |label_ctrl, field_ctrl, y, field_h, bg: &mut Scene| {
            let (fx, fy, ww) = if wide {
                (x + 126, y, fw - 142)
            } else {
                (x + inset, y + 23, fw - inset * 2)
            };
            place(
                label_ctrl,
                x + inset,
                y + if wide { 5 } else { 0 },
                if wide { 104 } else { fw - inset * 2 },
                20,
            );
            show(label_ctrl);
            let combo = field_ctrl == self.cmb_ai_backend
                || field_ctrl == self.cmb_chat_backend
                || field_ctrl == self.cmb_hymt_quant
                || field_ctrl == self.cmb_opacity;
            if combo {
                // The combo owns one complete face at the row's exact bounds.
                place_combo(field_ctrl, fx, fy, ww, field_h, 240);
            } else {
                bg.surface(BoxRect::new(fx, fy, ww, field_h), p.line, 7);
                bg.surface(
                    BoxRect::new(fx + 1, fy + 1, ww - 2, field_h - 2),
                    p.field,
                    6,
                );
                place(field_ctrl, fx + 7, fy + 2, ww - 14, field_h - 4);
            }
            show(field_ctrl);
        };
        let top = 82 - offset;
        match section {
            ID_SETTINGS_TRANS => {
                let service_h = if wide { 116 } else { 142 };
                bg.card(BoxRect::new(x, top, fw, service_h), p);
                place(self.chk_ai_enabled, x + inset, top + 16, fw - inset * 2, 28);
                show(self.chk_ai_enabled);
                bg.surface(
                    BoxRect::new(x + inset, top + 54, fw - inset * 2, 1),
                    p.line,
                    0,
                );
                form_row(
                    self.lbl_ai_backend,
                    self.cmb_ai_backend,
                    top + 68,
                    32,
                    &mut bg,
                );
                let next = top + service_h + 16;
                if built_in {
                    let panel_h = if wide { 256 } else { 282 };
                    bg.card(BoxRect::new(x, next, fw, panel_h), p);
                    form_row(
                        self.lbl_hymt_quant,
                        self.cmb_hymt_quant,
                        next + 18,
                        32,
                        &mut bg,
                    );
                    let actions = next + 18 + row_h + 8;
                    let button_w = (fw - inset * 2 - 8) / 2;
                    for (i, c) in [
                        self.btn_hymt_check,
                        self.btn_hymt_run,
                        self.btn_hymt_stop,
                        self.btn_hymt_delete,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        place(
                            c,
                            x + inset + (i % 2) as i32 * (button_w + 8),
                            actions + (i / 2) as i32 * 40,
                            button_w,
                            32,
                        );
                        show(c);
                    }
                    bg.surface(
                        BoxRect::new(x + inset, actions + 88, fw - inset * 2, 1),
                        p.line,
                        0,
                    );
                    place(
                        self.lbl_hymt_status,
                        x + inset,
                        actions + 102,
                        fw - inset * 2,
                        64,
                    );
                    show(self.lbl_hymt_status);
                } else {
                    let connection_h = row_h * 3 + 32;
                    bg.card(BoxRect::new(x, next, fw, connection_h), p);
                    for (i, (label_ctrl, field_ctrl)) in [
                        (self.lbl_ai_url, self.edit_ai_url),
                        (self.lbl_ai_key, self.edit_ai_key),
                        (self.lbl_ai_model, self.edit_ai_model),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        form_row(
                            label_ctrl,
                            field_ctrl,
                            next + 16 + i as i32 * row_h,
                            32,
                            &mut bg,
                        );
                    }
                    let b = SendMessageW(self.cmb_ai_backend, CB_GETCURSEL, 0, 0);
                    if b == 2 || b == 3 {
                        let fy = next + 16 + 2 * row_h + if wide { 0 } else { 23 };
                        let fx = if wide { x + 126 } else { x + inset };
                        let ww = if wide { fw - 142 } else { fw - inset * 2 };
                        place(self.edit_ai_model, fx + 7, fy + 2, (ww - 108).max(30), 28);
                        place(self.btn_ai_discover, x + fw - inset - 86, fy, 86, 32);
                        show(self.btn_ai_discover);
                    }
                    let prompt_y = next + connection_h + 16;
                    bg.card(BoxRect::new(x, prompt_y, fw, 152), p);
                    place(
                        self.lbl_ai_prompt,
                        x + inset,
                        prompt_y + 14,
                        fw - inset * 2,
                        20,
                    );
                    show(self.lbl_ai_prompt);
                    bg.surface(
                        BoxRect::new(x + inset, prompt_y + 44, fw - inset * 2, 92),
                        p.field,
                        7,
                    );
                    place(
                        self.edit_ai_prompt,
                        x + inset + 6,
                        prompt_y + 47,
                        fw - inset * 2 - 12,
                        86,
                    );
                    show(self.edit_ai_prompt);
                }
            }
            ID_SETTINGS_CHAT => {
                place(self.btn_chat_copy_trans, x + fw - 110, 20 - offset, 110, 30);
                show(self.btn_chat_copy_trans);
                bg.card(BoxRect::new(x, top, fw, if wide { 82 } else { 104 }), p);
                form_row(
                    self.settings_labels[0],
                    self.cmb_chat_backend,
                    top + 20,
                    32,
                    &mut bg,
                );
                let next = top + if wide { 98 } else { 120 };
                bg.card(BoxRect::new(x, next, fw, row_h * 3 + 32), p);
                for (i, c) in [self.edit_chat_url, self.edit_chat_key, self.edit_chat_model]
                    .into_iter()
                    .enumerate()
                {
                    form_row(
                        self.settings_labels[i + 1],
                        c,
                        next + 16 + i as i32 * row_h,
                        32,
                        &mut bg,
                    );
                }
            }
            ID_SETTINGS_KEYS => {
                let card_h = row_h * 5 + 32;
                bg.card(BoxRect::new(x, top, fw, card_h), p);
                for (i, c) in [
                    self.edit_hk_toggle,
                    self.edit_hk_ocr,
                    self.edit_hk_chat,
                    self.edit_hk_sel_trans,
                    self.edit_hk_sel_chat,
                ]
                .into_iter()
                .enumerate()
                {
                    form_row(
                        self.settings_labels[i + 4],
                        c,
                        top + 16 + i as i32 * row_h,
                        32,
                        &mut bg,
                    );
                }
                place(self.btn_hk_reset, x, top + card_h + 16, 126, 32);
                show(self.btn_hk_reset);
                bg.label(
                    BoxRect::new(x, top + card_h + 58, fw, 20),
                    "点击后按组合键；Backspace / Delete 清空，Esc 取消",
                    true,
                    true,
                );
            }
            _ => {
                let opacity_h = if wide { 84 } else { 112 };
                bg.card(BoxRect::new(x, top, fw, opacity_h + 364), p);
                form_row(self.lbl_opacity, self.cmb_opacity, top + 18, 32, &mut bg);
                for (i, (c, description)) in [
                    (
                        self.chk_normal_window,
                        "显示在任务栏和 Alt+Tab 中，切走时不收起",
                    ),
                    (self.chk_ocr_newline, "合并识别文本中的段落换行"),
                    (self.chk_autostart, "登录 Windows 时自动启动"),
                ]
                .into_iter()
                .enumerate()
                {
                    let y = top + opacity_h + i as i32 * 78;
                    bg.surface(BoxRect::new(x + inset, y, fw - inset * 2, 1), p.line, 0);
                    place(c, x + inset, y + 15, fw - inset * 2, 26);
                    show(c);
                    bg.label(
                        BoxRect::new(
                            x + inset + if c == self.chk_normal_window { 0 } else { 25 },
                            y + 44,
                            fw - inset * 2 - if c == self.chk_normal_window { 0 } else { 25 },
                            18,
                        ),
                        description,
                        true,
                        true,
                    );
                }
                let update_y = top + opacity_h + 246;
                place(self.btn_update, x + inset, update_y, 140, 32);
                place(
                    self.lbl_update,
                    x + inset,
                    update_y + 42,
                    fw - inset * 2,
                    64,
                );
                show(self.btn_update);
                show(self.lbl_update);
            }
        }
        scene(self.settings_view, bg);
        InvalidateRect(self.settings_view, ptr::null(), 0);
    }

    unsafe fn switch_tab(&mut self, tab: TabMode) {
        if self.active_tab == tab {
            return;
        }
        let leaving_settings = self.active_tab == TabMode::Settings && tab != TabMode::Settings;
        if leaving_settings {
            self.set_pinned(self.pin_before_settings);
        } else if tab == TabMode::Settings && self.active_tab != TabMode::Settings {
            self.pin_before_settings = self.pinned;
            self.settings_return_size = Some(self.client_size());
            self.pending_resize_save = None;
            self.set_pinned(true);
            self.populate_settings_fields();
        }
        self.active_tab = tab;
        if tab == TabMode::Settings {
            let (width, height) = self.client_size();
            SetWindowPos(
                self.hwnd,
                ptr::null_mut(),
                0,
                0,
                scale(width.max(640)),
                scale(height.max(704)),
                SWP_NOMOVE | SWP_NOACTIVATE | 4,
            );
        } else if leaving_settings {
            if let Some((width, height)) = self.settings_return_size.take() {
                SetWindowPos(
                    self.hwnd,
                    ptr::null_mut(),
                    0,
                    0,
                    scale(width),
                    scale(height),
                    SWP_NOMOVE | SWP_NOACTIVATE | 4,
                );
            }
        }
        self.relayout_children();
        match tab {
            TabMode::Translate => {
                SetFocus(self.edit_source);
            }
            TabMode::Chat => {
                self.render_chat_transcript();
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
        selection(self.active_tab, self.settings_section, pin);
        InvalidateRect(self.btn_pin, ptr::null(), 0);
    }

    unsafe fn apply_normal_window_mode(&self, enabled: bool) {
        let current = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
        let style = (current & !(WS_EX_APPWINDOW | WS_EX_TOOLWINDOW))
            | if enabled {
                WS_EX_APPWINDOW
            } else {
                WS_EX_TOOLWINDOW
            };
        if current == style {
            return;
        }
        let visible = IsWindowVisible(self.hwnd) != 0;
        let focus = GetFocus();
        // The shell updates taskbar membership when a window is hidden and shown.
        // Do not use hide_window here: switching modes must keep active requests.
        if visible {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, style as isize);
        SetWindowPos(
            self.hwnd,
            ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        if visible {
            ShowWindow(self.hwnd, SW_SHOW);
            SetForegroundWindow(self.hwnd);
            if !focus.is_null() {
                SetFocus(focus);
            }
        }
    }

    unsafe fn save_normal_window_mode(&self) {
        let mut cfg = self.settings.appearance();
        let previous = cfg.normal_window_mode;
        cfg.normal_window_mode =
            SendMessageW(self.chk_normal_window, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        if let Err(error) = self.settings.save_appearance(cfg) {
            SendMessageW(
                self.chk_normal_window,
                BM_SETCHECK,
                if previous { BST_CHECKED } else { BST_UNCHECKED },
                0,
            );
            set_hwnd_text(
                self.lbl_settings_status,
                &format!("保存窗口模式失败: {error}"),
            );
        } else {
            self.apply_normal_window_mode(cfg.normal_window_mode);
            set_hwnd_text(self.lbl_settings_status, "窗口模式已保存");
        }
        InvalidateRect(self.chk_normal_window, ptr::null(), 0);
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
        self.set_shortcut_recording(false);
        self.web_cancel.cancel_current();
        self.ai_cancel.cancel_current();
        ShowWindow(self.hwnd, SW_HIDE);
    }

    fn apply_shortcuts(&mut self, cfg: &ShortcutConfig) -> Result<(), String> {
        let parsed = parse_shortcut_config(cfg).map_err(|e| e.to_string())?;
        if !self.recording_shortcut {
            register_shortcuts(&self.hotkey_mgr, &mut self.registered_hotkeys, &parsed)?;
        }
        self.parsed_shortcuts = parsed;
        Ok(())
    }

    unsafe fn set_shortcut_recording(&mut self, recording: bool) {
        if recording == self.recording_shortcut {
            return;
        }
        self.recording_shortcut = recording;
        // Discard pending presses so recording never opens a window or starts OCR.
        while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
        let desired = if recording {
            ParsedShortcuts::default()
        } else {
            self.parsed_shortcuts.clone()
        };
        match register_shortcuts(&self.hotkey_mgr, &mut self.registered_hotkeys, &desired) {
            Ok(()) if recording => set_hwnd_text(
                self.lbl_settings_status,
                "请按组合键；Backspace / Delete 清空，Esc 取消",
            ),
            Ok(()) => set_hwnd_text(self.lbl_settings_status, "录入完成，保存设置后生效"),
            Err(error) => set_hwnd_text(self.lbl_settings_status, &error),
        }
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
            let res = ttree_translate::translate_web(
                &text_for_web,
                &src_for_web,
                &tgt_for_web,
                &web_token,
            )
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

    unsafe fn render_chat_transcript(&mut self) {
        let _frame = LayoutFrame::new(self.hwnd, self.edit_chat_history);
        let mut r: RECT = std::mem::zeroed();
        GetClientRect(self.edit_chat_history, &mut r);
        let w = unscale(r.right).max(200);
        let h = unscale(r.bottom).max(40);
        let p = palette();
        let mut scene_bg = Scene::default();
        let mut messages: Vec<(bool, String)> = self
            .current_messages
            .iter()
            .map(|m| (m.role == "user", m.content.clone()))
            .collect();
        if self.chat_streaming {
            messages.push((
                false,
                if self.streaming_reply.is_empty() {
                    "思考中…".into()
                } else {
                    self.streaming_reply.clone()
                },
            ));
        }
        let mut y = 38;
        let mut frames = Vec::new();
        let max_bubble_w = (w - 64).max(100);
        for (user, content) in &messages {
            let (text_w, text_h) =
                measure_bubble(self.edit_chat_history, content, max_bubble_w - 28);
            let bubble_w = (text_w + 32).min(max_bubble_w);
            let height = text_h + 24;
            let x = if *user { w - bubble_w - 18 } else { 18 };
            frames.push((x, y, bubble_w, height, *user));
            y += height + 38;
        }
        let follow = near_bottom(self.edit_chat_history);
        scroll_range(self.edit_chat_history, y + 8, h);
        if follow {
            scroll_bottom(self.edit_chat_history);
        }
        let offset = scroll_pos(self.edit_chat_history);
        while self.chat_bubbles.len() > messages.len() {
            if let Some(c) = self.chat_bubbles.pop() {
                forget_bubble(c);
                DestroyWindow(c);
            }
        }
        for (i, ((_, content), (x, y, width, height, user))) in
            messages.iter().zip(frames).enumerate()
        {
            if i >= self.chat_bubbles.len() {
                let ctrl = self.create_ctrl(
                    GetModuleHandleW(ptr::null()),
                    "EDIT",
                    "",
                    ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
                    700 + i,
                    false,
                );
                SetParent(ctrl, self.edit_chat_history);
                self.chat_bubbles.push(ctrl);
            }
            let c = self.chat_bubbles[i];
            bubble_style(c, user);
            if get_hwnd_text(c) != *content {
                set_hwnd_text(c, content);
            }
            place(c, x + 10, y + 4 - offset, width - 20, height - 8);
            ShowWindow(c, SW_SHOW);
            scene_bg.surface(
                BoxRect::new(x, y - offset, width, height),
                if user { p.field } else { p.surface },
                10,
            );
            let label_rect = BoxRect::new(x + 2, y - 23 - offset, width - 4, 18);
            if user {
                scene_bg.right_label(label_rect, "你");
            } else {
                scene_bg.label(label_rect, "AI", true, true);
            }
        }
        if messages.is_empty() {
            scene_bg.label(
                BoxRect::new(24, (h / 2 - 12).max(20), w - 48, 24),
                "输入消息，开始 AI 对话",
                false,
                true,
            );
        }
        scene(self.edit_chat_history, scene_bg);
        InvalidateRect(self.edit_chat_history, ptr::null(), 0);
        self.relayout_children();
    }

    unsafe fn layout_chat_scroll(&self, old_offset: i32) {
        let _frame = LayoutFrame::new(self.hwnd, self.edit_chat_history);
        let offset = scroll_pos(self.edit_chat_history);
        let delta = old_offset - offset;
        for c in &self.chat_bubbles {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(*c, &mut r);
            let mut parent: RECT = std::mem::zeroed();
            GetWindowRect(self.edit_chat_history, &mut parent);
            place(
                *c,
                unscale(r.left - parent.left),
                unscale(r.top - parent.top) + delta,
                unscale(r.right - r.left),
                unscale(r.bottom - r.top),
            );
        }
        shift_scene(self.edit_chat_history, delta);
        InvalidateRect(self.edit_chat_history, ptr::null(), 0);
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

    unsafe fn start_hymt_download(&mut self) {
        if self.hymt_removing {
            set_hwnd_text(self.lbl_hymt_status, "正在删除服务，请稍候…");
            return;
        }
        if self.hymt_downloading {
            self.hymt_cancel_requested = true;
            self.hymt_cancel.cancel_current();
            set_hwnd_text(self.lbl_hymt_status, "正在取消下载，已下载进度将保留…");
            return;
        }
        let q_idx = (SendMessageW(self.cmb_hymt_quant, CB_GETCURSEL, 0, 0) as usize)
            .min(HyMtQuant::ALL.len() - 1);
        let quant = HyMtQuant::ALL[q_idx];
        if !hy_mt_manager::is_model_downloaded(quant) {
            let title = format!("选择模型下载文件夹 · {}", quant.file_name());
            match dialogs::pick_model_folder(self.hwnd, &hy_mt_manager::models_dir(), &title) {
                Ok(Some(folder)) => {
                    if let Err(e) = hy_mt_manager::set_custom_models_dir(folder) {
                        set_hwnd_text(self.lbl_hymt_status, &e);
                        return;
                    }
                }
                Ok(None) => {
                    set_hwnd_text(self.lbl_hymt_status, "已取消选择模型下载文件夹，未开始下载");
                    return;
                }
                Err(e) => {
                    set_hwnd_text(self.lbl_hymt_status, &e);
                    return;
                }
            }
        }
        self.hymt_downloading = true;
        self.hymt_cancel_requested = false;
        set_hwnd_text(self.btn_hymt_run, "取消下载");
        set_hwnd_text(
            self.lbl_hymt_status,
            &format!(
                "正在准备 {}\n保存至：{}",
                quant.file_name(),
                hy_mt_manager::models_dir().display()
            ),
        );
        self.hymt_seq = self.hymt_seq.wrapping_add(1);
        let seq = self.hymt_seq;
        let token = self.hymt_cancel.next();
        let tx_prog = self.ui_tx.clone();
        let tx_done = self.ui_tx.clone();
        self.rt.spawn(async move {
            let res = hy_mt_manager::download_and_start(quant, &token, |p| {
                let _ = tx_prog.send(UiEvent::HyMtProgress { seq, progress: p });
            })
            .await;
            let _ = tx_done.send(UiEvent::HyMtReady { seq, result: res });
        });
    }

    unsafe fn cancel_builtin_requests(&mut self) {
        if self.settings.ai_translation_for_request().backend == AiBackendKind::BuiltInHyMt {
            self.ai_cancel.cancel_current();
            self.ignored_ai_seq = Some(self.translate_seq);
            set_hwnd_text(
                self.edit_ai_result,
                "内置模型服务已停止，请先在设置中启动服务",
            );
        }
        if self.settings.chat_ai_for_request().backend == AiBackendKind::BuiltInHyMt {
            self.chat_cancel.cancel_current();
            self.chat_seq = self.chat_seq.wrapping_add(1);
            self.chat_streaming = false;
            self.streaming_reply.clear();
            self.render_chat_transcript();
        }
    }
    unsafe fn stop_hymt_service(&mut self) {
        if self.hymt_removing {
            return;
        }
        self.hymt_cancel.cancel_current();
        self.hymt_seq = self.hymt_seq.wrapping_add(1);
        self.hymt_downloading = false;
        self.hymt_cancel_requested = false;
        self.cancel_builtin_requests();
        let result = hy_mt_manager::stop_service();
        self.refresh_hymt_run_title();
        set_hwnd_text(
            self.lbl_hymt_status,
            &match result {
                Ok(()) => "内置服务已停止，翻译不会自动重启。点击「启动服务」可重新运行。".into(),
                Err(e) => e,
            },
        );
    }
    unsafe fn refresh_hymt_run_title(&self) {
        if self.hymt_downloading {
            return;
        }
        let q = (SendMessageW(self.cmb_hymt_quant, CB_GETCURSEL, 0, 0) as usize)
            .min(HyMtQuant::ALL.len() - 1);
        set_hwnd_text(
            self.btn_hymt_run,
            if hy_mt_manager::is_model_downloaded(HyMtQuant::ALL[q]) {
                "启动服务"
            } else {
                "下载并运行"
            },
        );
    }
    unsafe fn delete_hymt_service(&mut self) {
        if self.hymt_removing {
            return;
        }
        if !dialogs::confirm_remove_service(self.hwnd, &hy_mt_manager::models_dir()) {
            return;
        }
        self.stop_hymt_service();
        self.hymt_removing = true;
        let seq = self.hymt_seq;
        set_hwnd_text(
            self.lbl_hymt_status,
            "正在删除内置模型、下载断点和托管依赖…",
        );
        let tx = self.ui_tx.clone();
        self.rt.spawn(async move {
            let result = hy_mt_manager::remove_service().await;
            let _ = tx.send(UiEvent::HyMtRemoved { seq, result });
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

        shortcut_recorder::set_value(self.edit_hk_toggle, &hk_cfg.toggle);
        shortcut_recorder::set_value(self.edit_hk_ocr, &hk_cfg.ocr);
        shortcut_recorder::set_value(self.edit_hk_chat, &hk_cfg.ai_dialog);
        shortcut_recorder::set_value(self.edit_hk_sel_trans, &hk_cfg.selection_translate);
        shortcut_recorder::set_value(self.edit_hk_sel_chat, &hk_cfg.selection_ai_dialog);

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
            self.chk_normal_window,
            BM_SETCHECK,
            if app_cfg.normal_window_mode {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
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
        self.refresh_hymt_run_title();
        if hy_mt_manager::is_service_stopped() && !self.hymt_downloading && !self.hymt_removing {
            set_hwnd_text(
                self.lbl_hymt_status,
                "内置服务已停止。点击「启动服务」或「下载并运行」后才会恢复。",
            );
        }
    }

    unsafe fn save_settings_from_ui(&mut self) {
        SetFocus(self.btn_save_settings);
        self.set_shortcut_recording(false);
        let ai_enabled =
            SendMessageW(self.chk_ai_enabled, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
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
            toggle: shortcut_recorder::value(self.edit_hk_toggle),
            ocr: shortcut_recorder::value(self.edit_hk_ocr),
            ai_dialog: shortcut_recorder::value(self.edit_hk_chat),
            selection_translate: shortcut_recorder::value(self.edit_hk_sel_trans),
            selection_ai_dialog: shortcut_recorder::value(self.edit_hk_sel_chat),
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
            normal_window_mode: self.settings.appearance().normal_window_mode,
        };
        if let Err(error) = self.settings.save_appearance(app_cfg) {
            set_hwnd_text(
                self.lbl_settings_status,
                &format!("保存通用设置失败: {error}"),
            );
            return;
        }
        let alpha = (opacity * 255.0).round() as u8;
        SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);

        let want_autostart =
            SendMessageW(self.chk_autostart, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        let _ = autostart::set_enabled(want_autostart);

        set_hwnd_text(self.lbl_settings_status, "设置已保存");
        self.relayout_children();
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
                if self.ignored_ai_seq == Some(seq) {
                    return;
                }
                if seq != self.translate_seq {
                    return;
                }
                self.ai_stream_buf.push_str(&delta);
                set_hwnd_text(self.edit_ai_result, &self.ai_stream_buf);
            }
            UiEvent::AiTranslateDone { seq, result } => {
                if self.ignored_ai_seq == Some(seq) {
                    return;
                }
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
                                let conv_id =
                                    self.conversations.get(self.active_conv_idx).map(|c| c.id);
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
            UiEvent::HyMtProgress {
                seq,
                progress: prog,
            } => {
                if seq != self.hymt_seq || self.hymt_removing {
                    return;
                }
                if self.hymt_cancel_requested {
                    return;
                }
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
            UiEvent::HyMtReady { seq, result: res } => {
                if seq != self.hymt_seq || self.hymt_removing {
                    return;
                }
                self.hymt_downloading = false;
                self.refresh_hymt_run_title();
                if self.hymt_cancel_requested {
                    let _ = hy_mt_manager::stop_service();
                    self.hymt_cancel_requested = false;
                    set_hwnd_text(self.lbl_hymt_status, "下载已取消，断点进度已保留");
                } else {
                    match res {
                        Ok(url) => set_hwnd_text(
                            self.lbl_hymt_status,
                            &format!("HY-MT1.5-1.8B 本地推理服务运行中 ({url})"),
                        ),
                        Err(err) => set_hwnd_text(
                            self.lbl_hymt_status,
                            &format!("HY-MT1.5 启动失败：{err}"),
                        ),
                    }
                }
            }
            UiEvent::HyMtRemoved { seq, result } => {
                if seq != self.hymt_seq {
                    return;
                }
                self.hymt_removing = false;
                self.refresh_hymt_run_title();
                set_hwnd_text(self.lbl_hymt_status,&match result{Ok(())=>"内置服务已删除：模型、下载断点、托管引擎及依赖已清理。可点击「下载并运行」重新安装。".into(),Err(e)=>format!("删除未完成：{e}")});
            }
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
        let view = self.updater.view();
        if view.exit_for_install {
            ttree_ai::hy_mt_manager::stop_server();
            DestroyWindow(self.hwnd);
            return;
        }
        if view != self.update_view {
            set_hwnd_text(self.btn_update, view.button);
            set_hwnd_text(self.lbl_update, &view.status);
            EnableWindow(self.btn_update, (!view.busy) as BOOL);
            self.update_view = view;
        }
        while let Ok(hk_ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if !self.recording_shortcut && hk_ev.state == HotKeyState::Pressed {
                if let Some(action) = self.parsed_shortcuts.action_for_id(hk_ev.id) {
                    self.handle_hotkey_action(action);
                }
            }
        }
        while let Ok(ui_ev) = self.ui_rx.try_recv() {
            self.handle_ui_event(ui_ev);
            self.relayout_children();
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
        WM_NCCALCSIZE => 0,
        WM_NCHITTEST => {
            let x = (lparam as u32 & 0xffff) as u16 as i16 as i32;
            let y = ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32;
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let edge = scale(5);
            let left = x < r.left + edge;
            let right = x >= r.right - edge;
            let top = y < r.top + edge;
            let bottom = y >= r.bottom - edge;
            if top && left {
                13
            } else if top && right {
                14
            } else if bottom && left {
                16
            } else if bottom && right {
                17
            } else if left {
                10
            } else if right {
                11
            } else if top {
                12
            } else if bottom {
                15
            } else if y < r.top + scale(TOOLBAR) {
                2
            } else {
                1
            }
        }
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => 1,
        WM_DRAWITEM => draw_item(lparam),
        WM_MEASUREITEM => measure_item(lparam),
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            if msg == WM_SETTINGCHANGE {
                set_tray_icon(hwnd, NIM_MODIFY);
            }
            with_state(|st| {
                let dpi = if msg == WM_DPICHANGED {
                    (wparam & 0xffff) as u32
                } else {
                    st.dpi
                };
                let old_regular = st.font_regular;
                let old_bold = st.font_bold;
                st.dpi = dpi.max(96);
                st.font_regular = font(13, 400, st.dpi);
                st.font_bold = font(13, 600, st.dpi);
                configure(st.dpi, st.font_regular, st.font_bold);
                EnumChildWindows(hwnd, refresh_control, 0);
                small_label(st.lbl_web_header);
                small_label(st.lbl_ai_header);
                DeleteObject(old_regular);
                DeleteObject(old_bold);
                let dark = palette().dark() as u32;
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &dark as *const _ as *const c_void,
                    4,
                );
                if msg == WM_DPICHANGED {
                    let r = &*(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        ptr::null_mut(),
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOACTIVATE | 4,
                    );
                }
                st.relayout_children();
                st.render_chat_transcript();
            });
            0
        }
        m if m == WM_APP + 2 => {
            with_state(|st| {
                if wparam == ID_SETTINGS_VIEW {
                    st.layout_settings();
                } else {
                    st.layout_chat_scroll(lparam as i32);
                }
            });
            0
        }
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
                    st.set_shortcut_recording(false);
                    if !st.pinned
                        && !st.settings.appearance().normal_window_mode
                        && !st.ocr_in_flight
                        && st.active_tab != TabMode::Settings
                    {
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
                    if st.active_tab == TabMode::Chat {
                        st.render_chat_transcript();
                    }
                    if st.active_tab != TabMode::Settings {
                        st.pending_resize_save = Some((
                            Instant::now() + RESIZE_SAVE_DEBOUNCE,
                            unscale(w) as f64,
                            unscale(h) as f64,
                        ));
                    }
                });
            }
            0
        }
        WM_GETMINMAXINFO => {
            let mmi = lparam as *mut MINMAXINFO;
            if !mmi.is_null() {
                (*mmi).ptMinTrackSize.x = scale(WINDOW_MIN_WIDTH as i32);
                let mut min_h = WINDOW_MIN_HEIGHT as i32;
                with_state(|st| {
                    if st.active_tab == TabMode::Settings {
                        min_h = 380;
                    } else if st.active_tab == TabMode::Translate
                        && st.settings.ai_translation_for_request().enabled
                    {
                        min_h = 304;
                    }
                });
                (*mmi).ptMinTrackSize.y = scale(min_h);
                (*mmi).ptMaxTrackSize.x = scale(WINDOW_MAX_WIDTH as i32);
                (*mmi).ptMaxTrackSize.y = scale(WINDOW_MAX_HEIGHT as i32);
            }
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            color_control(wparam as HDC, lparam as HWND)
        }
        WM_LBUTTONDOWN => {
            let y = unscale(((lparam as u32 >> 16) & 0xFFFF) as i16 as i32);
            with_state(|st| {
                if st.active_tab == TabMode::Translate {
                    let layout = st.translation_layout();
                    let s1 = layout.languages.y;
                    let s2 = layout.ai.y;
                    if (y - s1).abs() <= 4 {
                        st.dragging_splitter = Some((1, y));
                        SetCapture(hwnd);
                        SetCursor(LoadCursorW(ptr::null_mut(), IDC_SIZENS));
                    } else if layout.ai.h > 0 && (y - s2).abs() <= 4 {
                        st.dragging_splitter = Some((2, y));
                        SetCapture(hwnd);
                        SetCursor(LoadCursorW(ptr::null_mut(), IDC_SIZENS));
                    }
                }
            });
            0
        }
        WM_MOUSEMOVE => {
            let y = unscale(((lparam as u32 >> 16) & 0xFFFF) as i16 as i32);
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
        ID_BTN_UPDATE if code == BN_CLICKED => {
            st.updater.activate(&st.rt);
        }
        ID_BTN_TAB_TRANS if code == BN_CLICKED => st.switch_tab(TabMode::Translate),
        ID_BTN_TAB_CHAT if code == BN_CLICKED => st.switch_tab(TabMode::Chat),
        ID_BTN_TAB_SETTINGS if code == BN_CLICKED => st.switch_tab(TabMode::Settings),
        ID_SETTINGS_DONE if code == BN_CLICKED => st.switch_tab(TabMode::Translate),
        ID_CLOSE if code == BN_CLICKED => st.hide_window(),
        ID_HISTORY_TOGGLE if code == BN_CLICKED => {
            st.history_visible = !st.history_visible;
            st.relayout_children();
            st.render_chat_transcript();
        }
        ID_SETTINGS_TRANS..=ID_SETTINGS_GENERAL if code == BN_CLICKED => {
            st.settings_section = id;
            set_scroll(st.settings_view, 0);
            st.relayout_children();
        }
        ID_CMB_AI_BACKEND | ID_CMB_CHAT_BACKEND if code == CBN_SELCHANGE => {
            set_scroll(st.settings_view, 0);
            st.relayout_children();
        }
        ID_BTN_OCR if code == BN_CLICKED => st.trigger_ocr(),
        ID_BTN_PIN if code == BN_CLICKED => st.set_pinned(!st.pinned),
        ID_CHK_NORMAL_WINDOW if code == BN_CLICKED => st.save_normal_window_mode(),

        ID_EDIT_SOURCE if code == EN_CHANGE => {
            st.refresh_source_input();
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
            st.translate_seq = st.translate_seq.wrapping_add(1);
            st.web_result_text.clear();
            st.ai_stream_buf.clear();
            set_hwnd_text(st.edit_web_result, "译文显示在这里");
            set_hwnd_text(st.edit_ai_result, "AI 译文显示在这里");
            st.relayout_children();
            SetFocus(st.edit_source);
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

        ID_CMB_HYMT_QUANT if code == CBN_SELCHANGE => st.refresh_hymt_run_title(),
        ID_BTN_HYMT_CHECK if code == BN_CLICKED => {
            let tx = st.ui_tx.clone();
            st.rt.spawn_blocking(move || {
                let rep = hy_mt_manager::check_compatibility();
                let _ = tx.send(UiEvent::HyMtCompatReady(rep));
            });
        }
        ID_BTN_HYMT_RUN if code == BN_CLICKED => st.start_hymt_download(),
        ID_BTN_HYMT_STOP if code == BN_CLICKED => st.stop_hymt_service(),
        ID_BTN_HYMT_DELETE if code == BN_CLICKED => st.delete_hymt_service(),
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
            st.relayout_children();
        }
        ID_BTN_HK_RESET if code == BN_CLICKED => {
            let def = ShortcutConfig::default();
            shortcut_recorder::set_value(st.edit_hk_toggle, &def.toggle);
            shortcut_recorder::set_value(st.edit_hk_ocr, &def.ocr);
            shortcut_recorder::set_value(st.edit_hk_chat, &def.ai_dialog);
            shortcut_recorder::set_value(st.edit_hk_sel_trans, &def.selection_translate);
            shortcut_recorder::set_value(st.edit_hk_sel_chat, &def.selection_ai_dialog);
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
