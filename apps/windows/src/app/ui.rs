//! Native presentation only. Keeps Win32 edits/combos/buttons for IME, selection,
//! keyboard access and accessibility, with the same restrained surfaces as AppKit.
use super::*;
use crate::design::{logical, physical, BoxRect};
use lucide_icons::{Icon, LUCIDE_FONT_BYTES};
use std::collections::HashMap;

pub const WM_PAINT: u32 = 0x000f;
const WM_SETREDRAW: u32 = 0x000b;
pub const WS_EX_COMPOSITED: u32 = 0x02000000;
pub const WS_CLIPSIBLINGS: u32 = 0x04000000;
const WS_VISIBLE: u32 = 0x10000000;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_DRAWITEM: u32 = 0x002b;
pub const WM_MEASUREITEM: u32 = 0x002c;
pub const WM_DPICHANGED: u32 = 0x02e0;
pub const WM_SETTINGCHANGE: u32 = 0x001a;
pub const WM_NCCALCSIZE: u32 = 0x0083;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_VSCROLL: u32 = 0x0115;
pub const WM_MOUSEWHEEL: u32 = 0x020a;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_MOUSELEAVE: u32 = 0x02a3;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WS_TABSTOP: u32 = 0x00010000;
pub const BS_OWNERDRAW: u32 = 0x000b;
pub const CBS_OWNERDRAWFIXED: u32 = 0x0010;
pub const ID_SETTINGS_TRANS: usize = 501;
pub const ID_SETTINGS_CHAT: usize = 502;
pub const ID_SETTINGS_KEYS: usize = 503;
pub const ID_SETTINGS_GENERAL: usize = 504;
pub const ID_SETTINGS_DONE: usize = 505;
pub const ID_HISTORY_TOGGLE: usize = 506;
pub const ID_CLOSE: usize = 507;
pub const ID_SETTINGS_VIEW: usize = 508;
const WM_GETDLGCODE: u32 = 0x0087;
const CB_GETLBTEXT: u32 = 0x0148;
const CB_GETLBTEXTLEN: u32 = 0x0149;
const CB_SETITEMHEIGHT: u32 = 0x0153;
const CB_GETITEMHEIGHT: u32 = 0x0154;
const ODS_COMBOBOXEDIT: u32 = 0x1000;
const DT_CENTER: u32 = 1;
const DT_RIGHT: u32 = 2;
const DT_VCENTER: u32 = 4;
const DT_WORDBREAK: u32 = 0x10;
const DT_SINGLELINE: u32 = 0x20;
const DT_CALCRECT: u32 = 0x400;
const DT_NOPREFIX: u32 = 0x800;
const DT_END_ELLIPSIS: u32 = 0x8000;
const SRCCOPY: u32 = 0x00cc0020;

#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    erase: BOOL,
    paint: RECT,
    restore: BOOL,
    inc_update: BOOL,
    reserved: [u8; 32],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct DRAWITEMSTRUCT {
    kind: u32,
    id: u32,
    item: u32,
    action: u32,
    state: u32,
    hwnd: HWND,
    hdc: HDC,
    rect: RECT,
    data: usize,
}
#[repr(C)]
struct MEASUREITEMSTRUCT {
    kind: u32,
    id: u32,
    item: u32,
    width: u32,
    height: u32,
    data: usize,
}
#[repr(C)]
struct SCROLLINFO {
    size: u32,
    mask: u32,
    min: i32,
    max: i32,
    page: u32,
    pos: i32,
    track: i32,
}
#[repr(C)]
struct TRACKMOUSEEVENT {
    size: u32,
    flags: u32,
    hwnd: HWND,
    hover: u32,
}

#[link(name = "user32")]
extern "system" {
    fn BeginPaint(hwnd: HWND, paint: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, paint: *const PAINTSTRUCT) -> BOOL;
    fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
    fn DrawTextW(hdc: HDC, text: *const u16, count: i32, rect: *mut RECT, flags: u32) -> i32;
    fn DrawFocusRect(hdc: HDC, rect: *const RECT) -> BOOL;
    pub fn GetDpiForSystem() -> u32;
    pub fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetDlgCtrlID(hwnd: HWND) -> i32;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn SetViewportOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn RedrawWindow(hwnd: HWND, rect: *const RECT, region: HANDLE, flags: u32) -> BOOL;
    fn GetFocus() -> HWND;
    fn GetKeyState(key: i32) -> i16;
    pub fn GetParent(hwnd: HWND) -> HWND;
    pub fn SetParent(hwnd: HWND, parent: HWND) -> HWND;
    fn PostMessageW(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> BOOL;
    fn TrackMouseEvent(event: *mut TRACKMOUSEEVENT) -> BOOL;
    fn SetScrollInfo(hwnd: HWND, bar: i32, info: *const SCROLLINFO, redraw: BOOL) -> i32;
    fn GetScrollInfo(hwnd: HWND, bar: i32, info: *mut SCROLLINFO) -> BOOL;
    fn ShowScrollBar(hwnd: HWND, bar: i32, show: BOOL) -> BOOL;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    pub fn IsDialogMessageW(hwnd: HWND, msg: *mut MSG) -> BOOL;
    pub fn EnumChildWindows(
        hwnd: HWND,
        callback: unsafe extern "system" fn(HWND, LPARAM) -> BOOL,
        data: LPARAM,
    ) -> BOOL;
}
#[link(name = "gdi32")]
extern "system" {
    fn AddFontMemResourceEx(
        data: *const c_void,
        size: u32,
        reserved: *const c_void,
        count: *mut u32,
    ) -> HANDLE;
    fn RemoveFontMemResourceEx(handle: HANDLE) -> BOOL;
    fn SelectObject(hdc: HDC, object: HANDLE) -> HANDLE;
    pub fn DeleteObject(object: HANDLE) -> BOOL;
    fn CreatePen(style: i32, width: i32, color: u32) -> HANDLE;
    fn RoundRect(hdc: HDC, l: i32, t: i32, r: i32, b: i32, ew: i32, eh: i32) -> BOOL;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn CreateCompatibleBitmap(hdc: HDC, width: i32, height: i32) -> HANDLE;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn BitBlt(
        dest: HDC,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src: HDC,
        sx: i32,
        sy: i32,
        rop: u32,
    ) -> BOOL;
}
#[link(name = "comctl32")]
extern "system" {
    fn InitCommonControls();
    fn SetWindowSubclass(
        hwnd: HWND,
        callback: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT,
        id: usize,
        data: usize,
    ) -> BOOL;
    fn DefSubclassProc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}
#[link(name = "uxtheme")]
extern "system" {
    fn SetWindowTheme(hwnd: HWND, app: *const u16, id: *const u16) -> i32;
}
#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(
        key: isize,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
}

fn uses_light_theme(setting: &str, default_light: bool) -> bool {
    let mut light = u32::from(default_light);
    let mut len = 4;
    unsafe {
        RegGetValueW(
            0x80000001u32 as i32 as isize,
            wide_null("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide_null(setting).as_ptr(),
            0x10,
            ptr::null_mut(),
            &mut light as *mut _ as *mut c_void,
            &mut len,
        );
    }
    light != 0
}

pub fn taskbar_uses_light_theme() -> bool {
    uses_light_theme("SystemUsesLightTheme", false)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: u32,
    pub surface: u32,
    pub field: u32,
    pub pill: u32,
    pub hover: u32,
    pub line: u32,
    pub ink: u32,
    pub muted: u32,
    pub faint: u32,
}
impl Palette {
    pub fn system() -> Self {
        if !uses_light_theme("AppsUseLightTheme", true) {
            Self {
                bg: rgb(31, 32, 34),
                surface: rgb(36, 37, 39),
                field: rgb(43, 44, 47),
                pill: rgb(57, 58, 61),
                hover: rgb(50, 51, 54),
                line: rgb(54, 55, 58),
                ink: rgb(209, 210, 212),
                muted: rgb(143, 145, 149),
                faint: rgb(113, 115, 119),
            }
        } else {
            Self {
                bg: rgb(247, 247, 246),
                surface: rgb(241, 242, 241),
                field: rgb(236, 237, 236),
                pill: rgb(255, 255, 254),
                hover: rgb(230, 232, 230),
                line: rgb(224, 226, 224),
                ink: rgb(65, 69, 66),
                muted: rgb(126, 131, 127),
                faint: rgb(157, 162, 158),
            }
        }
    }
    pub fn dark(self) -> bool {
        self.bg & 255 < 100
    }
}

#[derive(Clone)]
pub struct Surface {
    rect: BoxRect,
    color: u32,
    radius: i32,
}
#[derive(Clone)]
pub struct Label {
    rect: BoxRect,
    text: String,
    small: bool,
    heading: bool,
    muted: bool,
    right: bool,
}
#[derive(Clone, Default)]
pub struct Scene {
    surfaces: Vec<Surface>,
    labels: Vec<Label>,
}
impl Scene {
    pub fn surface(&mut self, rect: BoxRect, color: u32, radius: i32) {
        self.surfaces.push(Surface {
            rect,
            color,
            radius,
        });
    }
    pub fn label(&mut self, rect: BoxRect, text: &str, small: bool, muted: bool) {
        self.labels.push(Label {
            rect,
            text: text.into(),
            small,
            heading: false,
            muted,
            right: false,
        });
    }
    pub fn heading(&mut self, rect: BoxRect, title: &str) {
        self.labels.push(Label {
            rect,
            text: title.into(),
            small: false,
            heading: true,
            muted: false,
            right: false,
        });
    }
    pub fn card(&mut self, rect: BoxRect, p: Palette) {
        self.surface(rect, p.line, 10);
        self.surface(
            BoxRect::new(rect.x + 1, rect.y + 1, rect.w - 2, rect.h - 2),
            p.pill,
            9,
        );
    }
    pub fn right_label(&mut self, rect: BoxRect, text: &str) {
        self.labels.push(Label {
            rect,
            text: text.into(),
            small: true,
            heading: false,
            muted: true,
            right: true,
        });
    }
    pub fn divider(&mut self, y: i32, w: i32, color: u32) {
        self.surface(BoxRect::new(0, y, w, 1), color, 0);
    }
    pub fn grip(&mut self, y: i32, w: i32, color: u32) {
        self.divider(y, w, color);
        self.surface(BoxRect::new((w - 26) / 2, y - 1, 26, 2), color, 1);
    }
}
struct Visuals {
    dpi: u32,
    palette: Palette,
    regular: HFONT,
    small: HFONT,
    body: HFONT,
    bold: HFONT,
    heading: HFONT,
    icons: HFONT,
    icon_resource: HANDLE,
    tooltip: HWND,
    tips: Vec<Vec<u16>>,
    backgrounds: HashMap<usize, usize>,
    placements: HashMap<usize, BoxRect>,
    ranges: HashMap<usize, (i32, i32, i32)>,
    layouts: HashMap<usize, LayoutState>,
    active: TabMode,
    section: usize,
    pinned: bool,
    hover: HWND,
    scenes: HashMap<usize, Scene>,
    scroll: HashMap<usize, (i32, i32)>,
    brushes: [HBRUSH; 4],
    bubble_user: HashMap<usize, bool>,
}
thread_local! {
    static VISUALS: RefCell<Visuals> = RefCell::new(Visuals { dpi: 96, palette: Palette::system(), regular: ptr::null_mut(), small: ptr::null_mut(), body: ptr::null_mut(), bold: ptr::null_mut(), heading:ptr::null_mut(),icons:ptr::null_mut(),icon_resource:ptr::null_mut(),tooltip:ptr::null_mut(),tips:Vec::new(),backgrounds:HashMap::new(), placements:HashMap::new(),ranges:HashMap::new(),layouts:HashMap::new(), active: TabMode::Translate, section: ID_SETTINGS_TRANS, pinned: false, hover: ptr::null_mut(), scenes: HashMap::new(), scroll: HashMap::new(), brushes: [ptr::null_mut();4], bubble_user: HashMap::new() });
}

pub fn scale(value: i32) -> i32 {
    VISUALS.with(|v| physical(value, v.borrow().dpi))
}
pub fn unscale(value: i32) -> i32 {
    VISUALS.with(|v| logical(value, v.borrow().dpi))
}
pub fn palette() -> Palette {
    VISUALS.with(|v| v.borrow().palette)
}
pub unsafe fn font(size: i32, weight: i32, dpi: u32) -> HFONT {
    CreateFontW(
        -physical(size, dpi),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        wide_null("Microsoft YaHei UI").as_ptr(),
    )
}
pub unsafe fn configure(dpi: u32, regular: HFONT, bold: HFONT) {
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        if !v.small.is_null() {
            DeleteObject(v.small);
            DeleteObject(v.body);
            DeleteObject(v.heading);
            DeleteObject(v.icons);
        }
        if v.icon_resource.is_null() {
            let mut count = 0;
            v.icon_resource = AddFontMemResourceEx(
                LUCIDE_FONT_BYTES.as_ptr() as *const c_void,
                LUCIDE_FONT_BYTES.len() as u32,
                ptr::null(),
                &mut count,
            );
            assert!(
                !v.icon_resource.is_null() && count > 0,
                "加载内置 Lucide 图标库失败"
            );
        }
        for brush in v.brushes {
            if !brush.is_null() {
                DeleteObject(brush);
            }
        }
        v.placements.clear();
        v.ranges.clear();
        v.dpi = dpi.max(96);
        v.palette = Palette::system();
        v.regular = regular;
        v.bold = bold;
        v.brushes = [
            CreateSolidBrush(v.palette.bg),
            CreateSolidBrush(v.palette.surface),
            CreateSolidBrush(v.palette.field),
            CreateSolidBrush(v.palette.pill),
        ];
        v.small = font(11, 400, v.dpi);
        v.body = font(15, 400, v.dpi);
        v.heading = font(18, 600, v.dpi);
        v.icons = CreateFontW(
            -physical(18, v.dpi),
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
            wide_null("lucide").as_ptr(),
        );
    });
}
pub unsafe fn cleanup() {
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        for h in [v.small, v.body, v.heading, v.icons]
            .into_iter()
            .chain(v.brushes)
        {
            if !h.is_null() {
                DeleteObject(h);
            }
        }
        if !v.tooltip.is_null() {
            DestroyWindow(v.tooltip);
            v.tooltip = ptr::null_mut();
            v.tips.clear();
        }
        v.small = ptr::null_mut();
        v.body = ptr::null_mut();
        v.brushes = [ptr::null_mut(); 4];
        if !v.icon_resource.is_null() {
            RemoveFontMemResourceEx(v.icon_resource);
            v.icon_resource = ptr::null_mut();
        }
    });
}
pub fn control_background(hwnd: HWND, index: usize) {
    VISUALS.with(|v| {
        v.borrow_mut().backgrounds.insert(hwnd as usize, index);
    });
}
pub fn bubble_style(hwnd: HWND, user: bool) {
    VISUALS.with(|v| {
        v.borrow_mut().bubble_user.insert(hwnd as usize, user);
    });
}
pub fn forget_bubble(hwnd: HWND) {
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        v.bubble_user.remove(&(hwnd as usize));
        v.placements.remove(&(hwnd as usize));
    });
}
pub fn shift_scene(hwnd: HWND, delta: i32) {
    VISUALS.with(|v| {
        if let Some(s) = v.borrow_mut().scenes.get_mut(&(hwnd as usize)) {
            for x in &mut s.surfaces {
                x.rect.y += delta;
            }
            for x in &mut s.labels {
                x.rect.y += delta;
            }
        }
    });
}
pub unsafe fn color_control(hdc: HDC, hwnd: HWND) -> LRESULT {
    let id = GetDlgCtrlID(hwnd) as usize;
    let (p, brush, index) = VISUALS.with(|v| {
        let v = v.borrow();
        let index = if let Some(index) = v.backgrounds.get(&(hwnd as usize)) {
            *index
        } else if let Some(user) = v.bubble_user.get(&(hwnd as usize)) {
            if *user {
                2
            } else {
                1
            }
        } else if matches!(id, ID_EDIT_SOURCE | ID_EDIT_CHAT_INPUT) {
            1
        } else if (401..=425).contains(&id) {
            2
        } else {
            0
        };
        (v.palette, v.brushes[index], index)
    });
    let color = match index {
        1 => p.surface,
        2 => p.field,
        3 => p.pill,
        _ => p.bg,
    };
    SetBkMode(hdc, OPAQUE);
    SetBkColor(hdc, color);
    let muted = id == 0
        || matches!(id, ID_EDIT_WEB_RESULT | ID_EDIT_AI_RESULT)
            && matches!(
                get_hwnd_text(hwnd).as_str(),
                "译文显示在这里" | "AI 译文显示在这里" | "翻译中…" | "AI 翻译中…"
            );
    SetTextColor(hdc, if muted { p.muted } else { p.ink });
    brush as LRESULT
}
pub unsafe extern "system" fn refresh_control(hwnd: HWND, _data: LPARAM) -> BOOL {
    let id = GetDlgCtrlID(hwnd) as usize;
    let f = VISUALS.with(|v| {
        let v = v.borrow();
        if matches!(
            id,
            ID_EDIT_SOURCE | ID_EDIT_WEB_RESULT | ID_EDIT_AI_RESULT | ID_EDIT_CHAT_INPUT
        ) || id >= 700
        {
            v.body
        } else {
            v.regular
        }
    });
    SendMessageW(hwnd, WM_SETFONT, f as WPARAM, 1);
    theme_control(hwnd);
    1
}
pub fn selection(tab: TabMode, section: usize, pinned: bool) {
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        v.active = tab;
        v.section = section;
        v.pinned = pinned;
    });
}
pub fn scene(hwnd: HWND, value: Scene) {
    VISUALS.with(|v| {
        v.borrow_mut().scenes.insert(hwnd as usize, value);
    });
}
/// Publish background geometry and native child positions as one frame. Nested
/// settings/chat layout joins the same transaction, retaining hidden-window state.
struct LayoutState {
    depth: usize,
    was_visible: bool,
    dirty: Vec<HWND>,
}
pub struct LayoutFrame {
    root: HWND,
}
impl LayoutFrame {
    pub unsafe fn new(root: HWND, dirty: HWND) -> Self {
        let visible = IsWindowVisible(root) != 0;
        let disable = VISUALS.with(|v| {
            let mut v = v.borrow_mut();
            let state = v.layouts.entry(root as usize).or_insert(LayoutState {
                depth: 0,
                was_visible: visible,
                dirty: Vec::new(),
            });
            let disable = state.depth == 0 && state.was_visible;
            state.depth += 1;
            if !state.dirty.contains(&dirty) {
                state.dirty.push(dirty);
            }
            disable
        });
        if disable {
            SendMessageW(root, WM_SETREDRAW, 0, 0);
        }
        Self { root }
    }
}
impl Drop for LayoutFrame {
    fn drop(&mut self) {
        unsafe {
            let completed = VISUALS.with(|v| {
                let mut v = v.borrow_mut();
                let state = v.layouts.get_mut(&(self.root as usize))?;
                state.depth -= 1;
                if state.depth == 0 {
                    v.layouts.remove(&(self.root as usize))
                } else {
                    None
                }
            });
            if let Some(state) = completed {
                if state.was_visible {
                    SendMessageW(self.root, WM_SETREDRAW, 1, 0);
                }
                let dirty = if state.dirty.contains(&self.root) {
                    vec![self.root]
                } else {
                    state.dirty
                };
                for target in dirty {
                    // Native edits and scrollbars also have nonclient pixels. Invalidate
                    // them after the scene is committed, never during individual moves.
                    RedrawWindow(
                        target,
                        ptr::null(),
                        ptr::null_mut(),
                        0x0001
                            | 0x0004
                            | 0x0080
                            | 0x0400
                            | if state.was_visible { 0x0100 } else { 0 },
                    );
                }
            }
        }
    }
}
pub unsafe fn shown(hwnd: HWND) -> bool {
    // WM_SETREDRAW temporarily removes WS_VISIBLE from the root. Inspect each
    // child's own flag rather than IsWindowVisible's inherited visibility.
    (GetWindowLongPtrW(hwnd, -16) as u32 & WS_VISIBLE) != 0
}
pub unsafe fn visible(hwnd: HWND, show: bool) {
    if shown(hwnd) != show {
        ShowWindow(hwnd, if show { SW_SHOW } else { SW_HIDE });
    }
}
pub unsafe fn edit_geometry(hwnd: HWND) {
    let id = GetDlgCtrlID(hwnd) as usize;
    if !matches!(
        id,
        ID_EDIT_SOURCE
            | ID_EDIT_WEB_RESULT
            | ID_EDIT_AI_RESULT
            | ID_EDIT_CHAT_INPUT
            | ID_EDIT_AI_PROMPT
    ) && id < 700
    {
        return;
    }
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    let multiline = GetWindowLongPtrW(hwnd, -16) as u32 & ES_MULTILINE != 0;
    if !multiline {
        return;
    }
    let format = RECT {
        left: scale(2),
        top: scale(5),
        right: (r.right - scale(4)).max(scale(4)),
        bottom: (r.bottom - scale(4)).max(scale(8)),
    };
    SendMessageW(hwnd, 0x00b4, 0, &format as *const _ as LPARAM);
    // Bubble edits intentionally scroll with their viewport and own no scrollbar.
    if id < 700 {
        let lines = SendMessageW(hwnd, 0x00ba, 0, 0) as i32;
        let need = lines * scale(20) > r.bottom - scale(10);
        let current = GetWindowLongPtrW(hwnd, -16) as u32 & WS_VSCROLL != 0;
        if current != need {
            ShowScrollBar(hwnd, 1, need as BOOL);
            GetClientRect(hwnd, &mut r);
            let adjusted = RECT {
                right: (r.right - scale(4)).max(scale(4)),
                ..format
            };
            SendMessageW(hwnd, 0x00b4, 0, &adjusted as *const _ as LPARAM);
        }
    }
}
pub unsafe fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    let rect = BoxRect::new(scale(x), scale(y), scale(w.max(1)), scale(h.max(1)));
    let old = VISUALS.with(|v| v.borrow_mut().placements.insert(hwnd as usize, rect));
    if old == Some(rect) {
        return;
    }
    // Copying old control pixels while scrolling produces ghosts at their previous
    // positions. The enclosing LayoutFrame repaints the complete committed scene.
    SetWindowPos(
        hwnd,
        ptr::null_mut(),
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        0x0004 | 0x0010 | 0x0008 | 0x0100,
    );
    if old.is_none_or(|r| r.w != rect.w || r.h != rect.h) {
        edit_geometry(hwnd);
    }
}
/// The native combo's window height includes its selection field plus internal
/// chrome; the requested window height is otherwise interpreted as popup height.
/// Match the complete visible face to the form row, rather than nesting a second
/// rounded control inside a separately painted 32px field.
pub unsafe fn place_combo(hwnd: HWND, x: i32, y: i32, w: i32, face_height: i32, popup_height: i32) {
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(hwnd, &mut r);
    let item = SendMessageW(hwnd, CB_GETITEMHEIGHT, usize::MAX, 0).max(0) as i32;
    let chrome = (r.bottom - r.top - item).clamp(0, scale(12));
    let desired = (scale(face_height) - chrome).max(scale(16));
    if item != desired {
        SendMessageW(hwnd, CB_SETITEMHEIGHT, usize::MAX, desired as LPARAM);
        VISUALS.with(|v| {
            v.borrow_mut().placements.remove(&(hwnd as usize));
        });
    }
    if SendMessageW(hwnd, CB_GETITEMHEIGHT, 0, 0) != desired as LRESULT {
        SendMessageW(hwnd, CB_SETITEMHEIGHT, 0, desired as LPARAM);
    }
    place(hwnd, x, y, w, popup_height);
}

pub unsafe fn init_control(hwnd: HWND, class: &str, id: usize) {
    VISUALS.with(|v| {
        v.borrow_mut().placements.remove(&(hwnd as usize));
    });
    SetWindowSubclass(
        hwnd,
        control_proc,
        1,
        if class == "COMBOBOX" { 1 } else { 0 },
    );
    theme_control(hwnd);
    if class == "COMBOBOX" {
        SendMessageW(hwnd, CB_SETITEMHEIGHT, usize::MAX, scale(24) as LPARAM);
        SendMessageW(hwnd, CB_SETITEMHEIGHT, 0, scale(26) as LPARAM);
    }
    if class == "EDIT" {
        SendMessageW(hwnd, 0x00c5, 0x100000, 0); // EM_SETLIMITTEXT, same limit for paste/IME
        SendMessageW(
            hwnd,
            0x00d3,
            3,
            ((scale(6) as u32) << 16 | scale(6) as u32) as LPARAM,
        );
    }
    let body = matches!(
        id,
        ID_EDIT_SOURCE | ID_EDIT_WEB_RESULT | ID_EDIT_AI_RESULT | ID_EDIT_CHAT_INPUT
    ) || id >= 700;
    if body {
        VISUALS.with(|v| {
            SendMessageW(hwnd, WM_SETFONT, v.borrow().body as WPARAM, 1);
        });
    }
}
pub unsafe fn small_label(hwnd: HWND) {
    VISUALS.with(|v| {
        SendMessageW(hwnd, WM_SETFONT, v.borrow().small as WPARAM, 1);
    });
}
pub unsafe fn theme_control(hwnd: HWND) {
    let theme = wide_null(if palette().dark() {
        "DarkMode_Explorer"
    } else {
        "Explorer"
    });
    SetWindowTheme(hwnd, theme.as_ptr(), ptr::null());
}
pub unsafe fn cue(hwnd: HWND, text: &str) {
    SendMessageW(hwnd, 0x1501, 1, wide_null(text).as_ptr() as LPARAM);
}

fn px(r: BoxRect) -> RECT {
    RECT {
        left: scale(r.x),
        top: scale(r.y),
        right: scale(r.x + r.w),
        bottom: scale(r.y + r.h),
    }
}
unsafe fn fill(hdc: HDC, r: RECT, color: u32, radius: i32) {
    let brush = CreateSolidBrush(color);
    if radius == 0 {
        FillRect(hdc, &r, brush);
    } else {
        let pen = CreatePen(0, 1, color);
        let old_b = SelectObject(hdc, brush);
        let old_p = SelectObject(hdc, pen);
        RoundRect(
            hdc,
            r.left,
            r.top,
            r.right,
            r.bottom,
            scale(radius * 2),
            scale(radius * 2),
        );
        SelectObject(hdc, old_b);
        SelectObject(hdc, old_p);
        DeleteObject(pen);
    }
    DeleteObject(brush);
}
unsafe fn text(hdc: HDC, s: &str, r: RECT, font: HFONT, color: u32, flags: u32) {
    let old = SelectObject(hdc, font);
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, color);
    let mut rect = r;
    let wide = wide_null(s);
    DrawTextW(
        hdc,
        wide.as_ptr(),
        (wide.len() - 1) as i32,
        &mut rect,
        flags | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
}
/// Every icon, including the window toolbar and settings rail, comes from Lucide.
unsafe fn glyph(hdc: HDC, kind: Icon, r: RECT, color: u32) {
    let f = VISUALS.with(|v| v.borrow().icons);
    text(
        hdc,
        &char::from(kind).to_string(),
        r,
        f,
        color,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}
fn toolbar_icon(id: usize) -> Option<Icon> {
    Some(match id {
        ID_BTN_OCR => Icon::ScanText,
        ID_BTN_PIN => Icon::Pin,
        ID_BTN_TAB_SETTINGS => Icon::Settings,
        ID_CLOSE => Icon::X,
        ID_BTN_SWAP_LANG => Icon::ArrowRight,
        ID_BTN_CHAT_NEW => Icon::SquarePen,
        ID_HISTORY_TOGGLE => Icon::History,
        _ => return None,
    })
}
fn navigation_icon(id: usize) -> Option<Icon> {
    Some(match id {
        ID_SETTINGS_TRANS => Icon::Languages,
        ID_SETTINGS_CHAT => Icon::MessageCircle,
        ID_SETTINGS_KEYS => Icon::Keyboard,
        ID_SETTINGS_GENERAL => Icon::SlidersHorizontal,
        _ => return None,
    })
}
#[derive(Clone, Copy)]
enum ButtonKind {
    Ghost,
    Secondary,
    Primary,
    Navigation,
    Segment,
}
fn button_kind(id: usize) -> ButtonKind {
    match id {
        ID_BTN_SAVE_SETTINGS => ButtonKind::Primary,
        ID_SETTINGS_TRANS..=ID_SETTINGS_GENERAL => ButtonKind::Navigation,
        ID_BTN_TAB_TRANS | ID_BTN_TAB_CHAT => ButtonKind::Segment,
        ID_BTN_HYMT_CHECK
        | ID_BTN_HYMT_RUN
        | ID_BTN_HYMT_STOP
        | ID_BTN_HYMT_DELETE
        | ID_BTN_AI_DISCOVER
        | ID_BTN_CHAT_COPY_TRANS
        | ID_BTN_HK_RESET => ButtonKind::Secondary,
        _ => ButtonKind::Ghost,
    }
}

#[repr(C)]
struct ToolInfo {
    size: u32,
    flags: u32,
    hwnd: HWND,
    id: usize,
    rect: RECT,
    instance: HINSTANCE,
    text: *mut u16,
    param: LPARAM,
    reserved: *mut c_void,
}
pub unsafe fn install_tooltips(root: HWND, buttons: &[(HWND, &str)]) {
    InitCommonControls();
    let window = CreateWindowExW(
        8,
        wide_null("tooltips_class32").as_ptr(),
        ptr::null(),
        0x80000000 | 1 | 2,
        0,
        0,
        0,
        0,
        root,
        ptr::null_mut(),
        GetModuleHandleW(ptr::null()),
        ptr::null_mut(),
    );
    assert!(!window.is_null(), "创建图标按钮提示失败");
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        v.tooltip = window;
        for (button, label) in buttons {
            let mut words = wide_null(label);
            let info = ToolInfo {
                size: std::mem::size_of::<ToolInfo>() as u32,
                flags: 0x11,
                hwnd: root,
                id: *button as usize,
                rect: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                instance: ptr::null_mut(),
                text: words.as_mut_ptr(),
                param: 0,
                reserved: ptr::null_mut(),
            };
            SendMessageW(window, 0x0432, 0, &info as *const _ as LPARAM);
            v.tips.push(words);
        }
        SendMessageW(window, 0x0418, 0, physical(280, v.dpi) as LPARAM);
    });
}

unsafe fn buffered(target: HDC, r: RECT, draw: impl FnOnce(HDC)) {
    let dc = CreateCompatibleDC(target);
    let bitmap =
        CreateCompatibleBitmap(target, (r.right - r.left).max(1), (r.bottom - r.top).max(1));
    if dc.is_null() || bitmap.is_null() {
        if !dc.is_null() {
            DeleteDC(dc);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        draw(target);
        return;
    }
    let old = SelectObject(dc, bitmap);
    SetViewportOrgEx(dc, -r.left, -r.top, ptr::null_mut());
    draw(dc);
    BitBlt(
        target,
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        dc,
        r.left,
        r.top,
        SRCCOPY,
    );
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
}
unsafe fn paint_control(hwnd: HWND, draw: impl FnOnce(HDC)) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let target = BeginPaint(hwnd, &mut ps);
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    buffered(target, r, draw);
    EndPaint(hwnd, &ps);
}
pub unsafe fn draw_item(lparam: LPARAM) -> LRESULT {
    let item = &*(lparam as *const DRAWITEMSTRUCT);
    if item.kind == 3 && item.state & ODS_COMBOBOXEDIT != 0 {
        // The closed face is rendered by draw_combo. Native selection/focus
        // redraws must not paint another inset rectangle over its rounded frame.
        InvalidateRect(item.hwnd, ptr::null(), 0);
        return 1;
    }
    buffered(item.hdc, item.rect, |dc| {
        let copy = DRAWITEMSTRUCT { hdc: dc, ..*item };
        render_item(&copy);
    });
    1
}
unsafe fn render_item(d: &DRAWITEMSTRUCT) {
    let p = palette();
    if d.kind == 3 {
        // ODT_COMBOBOX: selection face and native popup rows
        let selected = d.state & 1 != 0;
        fill(d.hdc, d.rect, if selected { p.hover } else { p.bg }, 0);
        if d.item != u32::MAX {
            let mut r = d.rect;
            r.left += scale(8);
            VISUALS.with(|v| {
                text(
                    d.hdc,
                    &combo_text(d.hwnd, d.item as usize),
                    r,
                    v.borrow().regular,
                    p.ink,
                    DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                )
            });
        }
        return;
    }
    VISUALS.with(|v| {
        let v = v.borrow();
        let id = d.id as usize;
        let active = match id {
            ID_BTN_TAB_TRANS => v.active == TabMode::Translate,
            ID_BTN_TAB_CHAT => v.active == TabMode::Chat,
            ID_BTN_PIN => v.pinned,
            ID_SETTINGS_TRANS..=ID_SETTINGS_GENERAL => v.section == id,
            _ => false,
        };
        let hover = v.hover == d.hwnd || d.state & 1 != 0;
        let kind = button_kind(id);
        let contextual = VISUALS.with(|v| v.borrow().backgrounds.get(&(d.hwnd as usize)).copied());
        let base = if let Some(index) = contextual {
            match index {
                1 => p.surface,
                2 => p.field,
                3 => p.pill,
                _ => p.bg,
            }
        } else if matches!(kind, ButtonKind::Segment | ButtonKind::Navigation)
            || matches!(
                id,
                ID_BTN_TRANS_NOW
                    | ID_BTN_CLEAR_TRANS
                    | ID_BTN_CHAT_COPY
                    | ID_BTN_CHAT_CLEAR
                    | ID_BTN_CHAT_SEND
                    | ID_BTN_CHAT_STOP
            )
        {
            p.surface
        } else {
            p.bg
        };
        fill(d.hdc, d.rect, base, 0);
        let mut r = d.rect;
        r.left += scale(1);
        r.right -= scale(1);
        r.top += scale(1);
        r.bottom -= scale(1);
        let mut color = if active || hover { p.ink } else { p.muted };
        match kind {
            ButtonKind::Primary => {
                fill(d.hdc, r, if hover { p.muted } else { p.ink }, 7);
                color = p.pill;
            }
            ButtonKind::Secondary => {
                fill(d.hdc, r, p.line, 7);
                r.left += scale(1);
                r.right -= scale(1);
                r.top += scale(1);
                r.bottom -= scale(1);
                fill(d.hdc, r, if hover { p.hover } else { p.surface }, 6);
                color = p.ink;
            }
            _ => {
                if active || hover {
                    fill(d.hdc, r, if active { p.pill } else { p.hover }, 7);
                }
            }
        }
        if let Some(kind) = toolbar_icon(id) {
            glyph(d.hdc, kind, d.rect, color);
        } else {
            let mut r = d.rect;
            let flags = if let Some(kind) = navigation_icon(id) {
                let mut ir = r;
                ir.left += scale(10);
                ir.right = ir.left + scale(18);
                glyph(d.hdc, kind, ir, color);
                r.left += scale(38);
                0
            } else {
                DT_CENTER
            };
            text(
                d.hdc,
                &get_hwnd_text(d.hwnd),
                r,
                if active { v.bold } else { v.regular },
                color,
                flags | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
        if d.state & 0x10 != 0 && d.state & 0x200 == 0 {
            let mut r = d.rect;
            r.left += scale(3);
            r.right -= scale(3);
            r.top += scale(3);
            r.bottom -= scale(3);
            DrawFocusRect(d.hdc, &r);
        }
    });
}
pub unsafe fn measure_item(lparam: LPARAM) -> LRESULT {
    (*(lparam as *mut MEASUREITEMSTRUCT)).height = scale(26) as u32;
    1
}
unsafe fn combo_text(hwnd: HWND, idx: usize) -> String {
    let n = SendMessageW(hwnd, CB_GETLBTEXTLEN, idx, 0);
    if !(0..=16384).contains(&n) {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    SendMessageW(hwnd, CB_GETLBTEXT, idx, buf.as_mut_ptr() as LPARAM);
    String::from_utf16_lossy(&buf[..n as usize])
}

pub unsafe fn paint(hwnd: HWND) -> LRESULT {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let target = BeginPaint(hwnd, &mut ps);
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    let hdc = CreateCompatibleDC(target);
    let bitmap = CreateCompatibleBitmap(target, r.right.max(1), r.bottom.max(1));
    let old = SelectObject(hdc, bitmap);
    let p = palette();
    fill(hdc, r, p.bg, 0);
    // Clone the scene before GDI calls so synchronous window messages never reborrow the UI state.
    let (scene, small, regular, heading) = VISUALS.with(|v| {
        let v = v.borrow();
        (
            v.scenes.get(&(hwnd as usize)).cloned().unwrap_or_default(),
            v.small,
            v.regular,
            v.heading,
        )
    });
    for s in scene.surfaces {
        fill(hdc, px(s.rect), s.color, s.radius);
    }
    for label in scene.labels {
        text(
            hdc,
            &label.text,
            px(label.rect),
            if label.heading {
                heading
            } else if label.small {
                small
            } else {
                regular
            },
            if label.muted { p.muted } else { p.ink },
            DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | if label.right { DT_RIGHT } else { 0 },
        );
    }
    BitBlt(target, 0, 0, r.right, r.bottom, hdc, 0, 0, SRCCOPY);
    SelectObject(hdc, old);
    DeleteObject(bitmap);
    DeleteDC(hdc);
    EndPaint(hwnd, &ps);
    0
}

pub unsafe fn scroll_range(hwnd: HWND, content: i32, viewport: i32) -> i32 {
    let max = (content - viewport).max(0);
    let pos = VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        let entry = v.scroll.entry(hwnd as usize).or_default();
        entry.0 = entry.0.clamp(0, max);
        entry.1 = max;
        entry.0
    });
    let info = SCROLLINFO {
        size: std::mem::size_of::<SCROLLINFO>() as u32,
        mask: 7,
        min: 0,
        max: content.max(1) - 1,
        page: viewport.max(1) as u32,
        pos,
        track: 0,
    };
    let range = (content, viewport, pos);
    let changed =
        VISUALS.with(|v| v.borrow_mut().ranges.insert(hwnd as usize, range) != Some(range));
    if changed {
        SetScrollInfo(hwnd, 1, &info, 0);
    }
    pos
}
pub fn near_bottom(hwnd: HWND) -> bool {
    VISUALS.with(|v| {
        v.borrow()
            .scroll
            .get(&(hwnd as usize))
            .is_none_or(|(pos, max)| max - pos <= 24)
    })
}
pub fn scroll_pos(hwnd: HWND) -> i32 {
    VISUALS.with(|v| v.borrow().scroll.get(&(hwnd as usize)).map_or(0, |x| x.0))
}
pub fn set_scroll(hwnd: HWND, pos: i32) {
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        let e = v.scroll.entry(hwnd as usize).or_default();
        e.0 = pos.clamp(0, e.1);
    });
}
pub fn scroll_bottom(hwnd: HWND) {
    VISUALS.with(|v| {
        let mut v = v.borrow_mut();
        if let Some(e) = v.scroll.get_mut(&(hwnd as usize)) {
            e.0 = e.1;
        }
    });
}
pub unsafe fn scroll_message(hwnd: HWND, msg: u32, w: WPARAM) {
    let old = scroll_pos(hwnd);
    let next = if msg == WM_MOUSEWHEEL {
        old - ((w >> 16) as u16 as i16 as i32) * 48 / 120
    } else {
        let mut info: SCROLLINFO = std::mem::zeroed();
        info.size = std::mem::size_of::<SCROLLINFO>() as u32;
        info.mask = 0x17;
        GetScrollInfo(hwnd, 1, &mut info);
        match w & 0xffff {
            0 => old - 24,
            1 => old + 24,
            2 => old - info.page as i32,
            3 => old + info.page as i32,
            4 | 5 => info.track,
            6 => 0,
            7 => i32::MAX,
            _ => old,
        }
    };
    set_scroll(hwnd, next);
}
pub unsafe fn measure_bubble(hwnd: HWND, text_value: &str, max_w: i32) -> (i32, i32) {
    let hdc = GetDC(hwnd);
    let font = VISUALS.with(|v| v.borrow().body);
    let old = SelectObject(hdc, font);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: scale(max_w),
        bottom: 0,
    };
    let s = wide_null(text_value);
    DrawTextW(
        hdc,
        s.as_ptr(),
        (s.len() - 1) as i32,
        &mut rect,
        DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | 0x2000,
    );
    SelectObject(hdc, old);
    ReleaseDC(hwnd, hdc);
    (
        unscale(rect.right).clamp(20, max_w),
        unscale(rect.bottom).max(20),
    )
}

unsafe fn draw_checkbox(hwnd: HWND, hdc: HDC) {
    let p = palette();
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    let background = VISUALS.with(|v| match v.borrow().backgrounds.get(&(hwnd as usize)) {
        Some(3) => p.pill,
        _ => p.bg,
    });
    fill(hdc, r, background, 0);
    let center = (r.top + r.bottom) / 2;
    let check = SendMessageW(hwnd, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
    let box_r = RECT {
        left: scale(1),
        top: center - scale(7),
        right: scale(15),
        bottom: center + scale(7),
    };
    fill(hdc, box_r, p.line, 3);
    let inside = RECT {
        left: box_r.left + scale(1),
        top: box_r.top + scale(1),
        right: box_r.right - scale(1),
        bottom: box_r.bottom - scale(1),
    };
    fill(hdc, inside, if check { p.muted } else { p.pill }, 2);
    if check {
        glyph(hdc, Icon::Check, box_r, p.pill);
    }
    r.left += scale(25);
    let f = VISUALS.with(|v| v.borrow().regular);
    text(
        hdc,
        &get_hwnd_text(hwnd),
        r,
        f,
        p.ink,
        DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    if GetFocus() == hwnd {
        DrawFocusRect(hdc, &r);
    }
}

unsafe fn draw_combo(hwnd: HWND, hdc: HDC) {
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    let p = palette();
    let id = GetDlgCtrlID(hwnd) as usize;
    let flat = matches!(
        id,
        ID_CMB_SOURCE_LANG | ID_CMB_TARGET_LANG | ID_CMB_CHAT_CONV
    );
    let focus_visible = GetFocus() == hwnd && SendMessageW(hwnd, 0x0129, 0, 0) & 1 == 0;
    fill(hdc, r, if flat { p.surface } else { p.pill }, 0);
    if !flat || focus_visible {
        fill(hdc, r, if focus_visible { p.muted } else { p.line }, 7);
        let inner = RECT {
            left: r.left + scale(1),
            top: r.top + scale(1),
            right: r.right - scale(1),
            bottom: r.bottom - scale(1),
        };
        fill(hdc, inner, if flat { p.surface } else { p.field }, 6);
    }
    let mut tr = r;
    tr.left += scale(if flat { 8 } else { 12 });
    tr.right -= scale(32);
    let idx = SendMessageW(hwnd, CB_GETCURSEL, 0, 0);
    let label = if idx >= 0 {
        combo_text(hwnd, idx as usize)
    } else {
        String::new()
    };
    let f = VISUALS.with(|v| v.borrow().regular);
    text(
        hdc,
        &label,
        tr,
        f,
        p.ink,
        DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    let mut arrow = r;
    arrow.left = arrow.right - scale(24);
    arrow.right -= scale(4);
    glyph(hdc, Icon::ChevronDown, arrow, p.muted);
}

unsafe extern "system" fn control_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    combo: usize,
) -> LRESULT {
    let checkbox = matches!(
        GetDlgCtrlID(hwnd) as usize,
        ID_CHK_AI_ENABLED | ID_CHK_OCR_NEWLINE | ID_CHK_AUTOSTART
    );
    let interactive =
        checkbox || combo != 0 || GetWindowLongPtrW(hwnd, -16) as u32 & 0xf == BS_OWNERDRAW;
    match msg {
        0x000c => {
            let result = DefSubclassProc(hwnd, msg, w, l);
            edit_geometry(hwnd);
            return result;
        }
        WM_PAINT if checkbox => {
            paint_control(hwnd, |dc| draw_checkbox(hwnd, dc));
            return 0;
        }
        0x0318 if checkbox => {
            draw_checkbox(hwnd, w as HDC);
            return 0;
        }
        WM_ERASEBKGND if interactive => return 1,
        WM_MOUSEMOVE if interactive => {
            let changed = VISUALS.with(|v| {
                let mut v = v.borrow_mut();
                let old = v.hover;
                if old == hwnd {
                    None
                } else {
                    v.hover = hwnd;
                    Some(old)
                }
            });
            if let Some(old) = changed {
                let mut t = TRACKMOUSEEVENT {
                    size: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    flags: 2,
                    hwnd,
                    hover: 0,
                };
                TrackMouseEvent(&mut t);
                if !old.is_null() {
                    InvalidateRect(old, ptr::null(), 0);
                }
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        WM_MOUSELEAVE if interactive => {
            let changed = VISUALS.with(|v| {
                let mut v = v.borrow_mut();
                if v.hover == hwnd {
                    v.hover = ptr::null_mut();
                    true
                } else {
                    false
                }
            });
            if changed {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        WM_MOUSEWHEEL
            if GetDlgCtrlID(GetParent(hwnd)) as usize == ID_SETTINGS_VIEW
                && GetDlgCtrlID(hwnd) as usize != ID_EDIT_AI_PROMPT
                && (combo == 0 || SendMessageW(hwnd, 0x0157, 0, 0) == 0) =>
        {
            return SendMessageW(GetParent(hwnd), msg, w, l);
        }
        WM_MOUSEWHEEL if GetDlgCtrlID(hwnd) >= 700 => {
            return SendMessageW(GetParent(hwnd), msg, w, l);
        }
        WM_KEYDOWN if w == 0x1b => {
            if combo != 0 && SendMessageW(hwnd, 0x0157, 0, 0) != 0 {
                return DefSubclassProc(hwnd, msg, w, l);
            }
            PostMessageW(root(hwnd), WM_CLOSE, 0, 0);
            return 0;
        }
        WM_KEYDOWN
            if w == 0x0d
                && matches!(
                    GetDlgCtrlID(hwnd) as usize,
                    ID_EDIT_SOURCE | ID_EDIT_CHAT_INPUT
                )
                && GetKeyState(0x10) >= 0 =>
        {
            let id = if GetDlgCtrlID(hwnd) as usize == ID_EDIT_SOURCE {
                ID_BTN_TRANS_NOW
            } else {
                ID_BTN_CHAT_SEND
            };
            PostMessageW(root(hwnd), WM_COMMAND, id, 0);
            return 0;
        }
        WM_CHAR
            if w == 0x0d
                && matches!(
                    GetDlgCtrlID(hwnd) as usize,
                    ID_EDIT_SOURCE | ID_EDIT_CHAT_INPUT
                )
                && GetKeyState(0x10) >= 0 =>
        {
            return 0
        }
        WM_KEYDOWN if w == 0x41 && GetKeyState(0x11) < 0 && combo == 0 => {
            SendMessageW(hwnd, EM_SETSEL, 0, -1);
            return 0;
        }
        WM_GETDLGCODE
            if matches!(
                GetDlgCtrlID(hwnd) as usize,
                ID_EDIT_SOURCE | ID_EDIT_CHAT_INPUT | ID_EDIT_AI_PROMPT
            ) =>
        {
            return DefSubclassProc(hwnd, msg, w, l) | 4;
        }
        WM_PAINT
            if combo == 0
                && matches!(
                    GetDlgCtrlID(hwnd) as usize,
                    ID_EDIT_SOURCE | ID_EDIT_CHAT_INPUT
                )
                && get_hwnd_text(hwnd).is_empty() =>
        {
            let result = DefSubclassProc(hwnd, msg, w, l);
            let hdc = GetDC(hwnd);
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut r);
            r.left += scale(8);
            r.top += scale(5);
            r.bottom = r.top + scale(24);
            let label = if GetDlgCtrlID(hwnd) as usize == ID_EDIT_SOURCE {
                "输入文字，回车翻译"
            } else {
                "输入消息，回车发送"
            };
            let f = VISUALS.with(|v| v.borrow().body);
            text(
                hdc,
                label,
                r,
                f,
                palette().faint,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
            ReleaseDC(hwnd, hdc);
            return result;
        }
        WM_PAINT if combo != 0 => {
            paint_control(hwnd, |dc| draw_combo(hwnd, dc));
            return 0;
        }
        0x0318 if combo != 0 => {
            draw_combo(hwnd, w as HDC);
            return 0;
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, w, l)
}
unsafe fn root(mut hwnd: HWND) -> HWND {
    loop {
        let p = GetParent(hwnd);
        if p.is_null() {
            return hwnd;
        }
        hwnd = p;
    }
}

pub unsafe extern "system" fn viewport_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => 1,
        WM_VSCROLL | WM_MOUSEWHEEL => {
            let old = scroll_pos(hwnd);
            scroll_message(hwnd, msg, w);
            if scroll_pos(hwnd) != old {
                SendMessageW(
                    GetParent(hwnd),
                    WM_APP + 2,
                    GetDlgCtrlID(hwnd) as usize,
                    old as LPARAM,
                );
            }
            0
        }
        WM_COMMAND | WM_DRAWITEM | WM_MEASUREITEM | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC
        | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX => SendMessageW(GetParent(hwnd), msg, w, l),
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
