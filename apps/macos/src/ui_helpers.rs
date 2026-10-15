//! macOS 原生 AppKit (`objc2-app-kit`) 极简淡雅色调控件与可拖拽分栏辅助函数。

#![cfg(target_os = "macos")]
#![allow(clippy::too_many_arguments)]

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBox, NSBoxType, NSButton, NSButtonType, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSCursor, NSEvent, NSFont, NSImage, NSPasteboard,
    NSPasteboardTypeString, NSPopUpButton, NSScrollView, NSSecureTextField, NSSlider,
    NSTextAlignment, NSTextField, NSTextView, NSView,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString};

pub const SPLITTER_HIT_HEIGHT: f64 = 7.0;

/// 生成低饱和、柔和淡雅的自适应文本/线条颜色（严禁使用突兀的高对比或高饱和强调色）。
pub fn pale_ink(alpha: f64) -> Retained<NSColor> {
    NSColor::labelColor().colorWithAlphaComponent(alpha as CGFloat)
}

/// 将以顶部为原点的 `(x, top_y, width, height)` 转换为 AppKit 默认左下角原点的 `NSRect`。
pub fn top_rect(parent_height: f64, x: f64, top_y: f64, width: f64, height: f64) -> NSRect {
    let y = (parent_height - top_y - height).max(0.0);
    NSRect::new(
        NSPoint::new(x.max(0.0), y),
        NSSize::new(width.max(1.0), height.max(1.0)),
    )
}

pub struct FlippedViewIvars;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = FlippedViewIvars]
    #[name = "TTreeFlippedView"]
    pub struct FlippedView;

    unsafe impl NSObjectProtocol for FlippedView {}

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }
    }
);

/// 创建以左上角为原点的容器视图（用于 AI 对话左右气泡流式滚动列表）。
pub fn make_flipped_view(mtm: MainThreadMarker, frame: NSRect) -> Retained<FlippedView> {
    let v = mtm.alloc::<FlippedView>().set_ivars(FlippedViewIvars);
    unsafe { msg_send![super(v), initWithFrame: frame] }
}

/// 移除父视图内的全部子视图。
pub fn clear_subviews(view: &NSView) {
    loop {
        let subviews = view.subviews();
        if subviews.is_empty() {
            break;
        }
        if let Some(first) = subviews.firstObject() {
            first.removeFromSuperview();
        } else {
            break;
        }
    }
}

pub struct SplitHandleIvars {
    splitter_id: Cell<u8>,
    last_screen_y: Cell<f64>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SplitHandleIvars]
    #[name = "TTreeSplitHandleView"]
    pub struct SplitHandleView;

    unsafe impl NSObjectProtocol for SplitHandleView {}

    impl SplitHandleView {
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> Bool {
            Bool::NO
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> Bool {
            Bool::YES
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let bounds = self.bounds();
            #[allow(deprecated)]
            let cursor = NSCursor::resizeUpDownCursor();
            self.addCursorRect_cursor(bounds, &cursor);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let loc = NSEvent::mouseLocation();
            self.ivars().last_screen_y.set(loc.y);
            #[allow(deprecated)]
            NSCursor::resizeUpDownCursor().push();
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            let loc = NSEvent::mouseLocation();
            let prev_y = self.ivars().last_screen_y.get();
            let delta_down = prev_y - loc.y;
            if delta_down.abs() >= 0.5 {
                self.ivars().last_screen_y.set(loc.y);
                let id = self.ivars().splitter_id.get();
                crate::dispatch_splitter_drag(id, delta_down);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            NSCursor::pop_class();
            crate::dispatch_splitter_drag_end();
        }
    }
);

/// 可拖拽的水平分栏手柄组件（含命中区、1px 极淡分隔线、中央微弱握把）。
pub struct SplitBarWidget {
    pub handle: Retained<SplitHandleView>,
    pub line: Retained<NSBox>,
    pub grip: Retained<NSBox>,
}

/// 创建可上下拖拽调整栏高的水平分栏条 (`splitter_id` 标识对应分栏)。
pub fn make_split_bar(mtm: MainThreadMarker, frame: NSRect, splitter_id: u8) -> SplitBarWidget {
    let handle = mtm.alloc::<SplitHandleView>().set_ivars(SplitHandleIvars {
        splitter_id: Cell::new(splitter_id),
        last_screen_y: Cell::new(0.0),
    });
    let handle: Retained<SplitHandleView> =
        unsafe { msg_send![super(handle), initWithFrame: frame] };

    let line = make_divider(
        mtm,
        NSRect::new(
            NSPoint::new(0.0, (frame.size.height - 1.0) * 0.5),
            NSSize::new(frame.size.width, 1.0),
        ),
    );
    let grip_w = 26.0;
    let grip_h = 2.0;
    let grip = NSBox::initWithFrame(
        mtm.alloc(),
        NSRect::new(
            NSPoint::new(
                ((frame.size.width - grip_w) * 0.5).max(0.0),
                ((frame.size.height - grip_h) * 0.5).max(0.0),
            ),
            NSSize::new(grip_w, grip_h),
        ),
    );
    grip.setBoxType(NSBoxType::Custom);
    grip.setBorderWidth(0.0);
    grip.setCornerRadius(1.0);
    grip.setFillColor(&pale_ink(0.11));
    grip.setContentViewMargins(CGSize::new(0.0, 0.0));

    handle.addSubview(&line);
    handle.addSubview(&grip);

    SplitBarWidget { handle, line, grip }
}

/// 更新水平分栏条及其内部细线与握把的位置。
pub fn set_split_bar_frame(bar: &SplitBarWidget, frame: NSRect) {
    bar.handle.setFrame(frame);
    let h = frame.size.height;
    let w = frame.size.width;
    bar.line.setFrame(NSRect::new(
        NSPoint::new(0.0, ((h - 1.0) * 0.5).max(0.0)),
        NSSize::new(w.max(1.0), 1.0),
    ));
    let grip_w = 26.0;
    let grip_h = 2.0;
    bar.grip.setFrame(NSRect::new(
        NSPoint::new(((w - grip_w) * 0.5).max(0.0), ((h - grip_h) * 0.5).max(0.0)),
        NSSize::new(grip_w, grip_h),
    ));
    if let Some(win) = bar.handle.window() {
        win.invalidateCursorRectsForView(&bar.handle);
    }
}

/// 创建静态文本标签 (`NSTextField` 只读无边框，采用柔和淡雅字体色)。
pub fn make_label(
    mtm: MainThreadMarker,
    frame: NSRect,
    text: &str,
    font_size: f64,
    bold: bool,
    secondary: bool,
) -> Retained<NSTextField> {
    let label = NSTextField::initWithFrame(mtm.alloc(), frame);
    let ns_text = NSString::from_str(text);
    label.setStringValue(&ns_text);
    label.setBezeled(false);
    label.setDrawsBackground(false);
    label.setEditable(false);
    label.setSelectable(false);
    let font = if bold {
        NSFont::boldSystemFontOfSize(font_size as CGFloat)
    } else {
        NSFont::systemFontOfSize(font_size as CGFloat)
    };
    label.setFont(Some(&font));
    let color = if secondary {
        pale_ink(0.38)
    } else {
        pale_ink(0.72)
    };
    label.setTextColor(Some(&color));
    label
}

/// 创建 1px 极淡分隔线。
pub fn make_divider(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    let line = NSBox::initWithFrame(mtm.alloc(), frame);
    line.setBoxType(NSBoxType::Custom);
    line.setBorderWidth(0.0);
    line.setCornerRadius(0.0);
    line.setFillColor(&pale_ink(0.055));
    line.setContentViewMargins(CGSize::new(0.0, 0.0));
    line
}

/// 创建极淡半透明色块背景容器 (`NSBox` 无边框，将传入 alpha 自动缩放到柔和区间)。
pub fn make_surface_box(
    mtm: MainThreadMarker,
    frame: NSRect,
    corner_radius: f64,
    alpha: f64,
) -> Retained<NSBox> {
    let surface = NSBox::initWithFrame(mtm.alloc(), frame);
    surface.setBoxType(NSBoxType::Custom);
    surface.setBorderWidth(0.0);
    surface.setCornerRadius(corner_radius as CGFloat);
    let soft_alpha = (alpha * 0.08).clamp(0.015, 0.06);
    surface.setFillColor(&pale_ink(soft_alpha));
    surface.setContentViewMargins(CGSize::new(0.0, 0.0));
    surface
}

/// 创建柔和淡色圆角按钮 (`NSButton`，无突兀高亮蓝色)。
pub fn make_button(
    mtm: MainThreadMarker,
    frame: NSRect,
    title: &str,
    tag: isize,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSButton> {
    unsafe {
        let btn = NSButton::initWithFrame(mtm.alloc(), frame);
        btn.setBordered(false);
        btn.setWantsLayer(true);
        btn.setTitle(&NSString::from_str(title));
        btn.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        let tint = pale_ink(0.68);
        let _: () = msg_send![&*btn, setContentTintColor: Some(&*tint)];
        if let Some(layer) = btn.layer() {
            layer.setCornerRadius(6.0);
            let bg = pale_ink(0.05);
            let cg_bg: *const AnyObject = msg_send![&*bg, CGColor];
            let _: () = msg_send![&*layer, setBackgroundColor: cg_bg];
        }
        btn.setTag(tag);
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn
    }
}

/// 创建无边框扁平文字按钮（全部使用淡雅灰白/灰黑层级，不使用高亮色）。
pub fn make_flat_button(
    mtm: MainThreadMarker,
    frame: NSRect,
    title: &str,
    font_size: f64,
    bold: bool,
    accent: bool,
    tag: isize,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSButton> {
    unsafe {
        let btn = NSButton::initWithFrame(mtm.alloc(), frame);
        btn.setBordered(false);
        btn.setTitle(&NSString::from_str(title));
        let font = if bold {
            NSFont::boldSystemFontOfSize(font_size as CGFloat)
        } else {
            NSFont::systemFontOfSize(font_size as CGFloat)
        };
        btn.setFont(Some(&font));
        let color = if accent {
            pale_ink(0.62)
        } else {
            pale_ink(0.42)
        };
        let _: () = msg_send![&*btn, setContentTintColor: Some(&*color)];
        btn.setTag(tag);
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn
    }
}

/// 创建右上角功能区 SF Symbols 图标按钮（淡雅色调）。
pub fn make_symbol_button(
    mtm: MainThreadMarker,
    frame: NSRect,
    symbol_name: &str,
    fallback_title: &str,
    tooltip: &str,
    tag: isize,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSButton> {
    unsafe {
        let btn = NSButton::initWithFrame(mtm.alloc(), frame);
        btn.setBordered(false);
        btn.setToolTip(Some(&NSString::from_str(tooltip)));
        set_button_symbol(&btn, symbol_name, fallback_title, false);
        btn.setTag(tag);
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn
    }
}

/// 更新图标按钮的 SF Symbol 与淡雅激活态颜色（不使用刺眼蓝色）。
pub fn set_button_symbol(btn: &NSButton, symbol_name: &str, fallback_title: &str, active: bool) {
    unsafe {
        let ns_sym = NSString::from_str(symbol_name);
        let img: Option<Retained<NSImage>> = msg_send![
            NSImage::class(),
            imageWithSystemSymbolName: &*ns_sym,
            accessibilityDescription: None::<&NSString>
        ];
        if let Some(img) = img {
            btn.setImage(Some(&img));
            btn.setTitle(&NSString::from_str(""));
        } else {
            btn.setTitle(&NSString::from_str(fallback_title));
        }
        let tint = if active {
            pale_ink(0.75)
        } else {
            pale_ink(0.40)
        };
        let _: () = msg_send![btn, setContentTintColor: Some(&*tint)];
    }
}

/// 设置扁平按钮的文字颜色（用于激活态切换，保持淡雅克制）。
pub fn set_flat_button_active(btn: &NSButton, active: bool, _is_accent: bool) {
    unsafe {
        let tint = if active {
            pale_ink(0.76)
        } else {
            pale_ink(0.38)
        };
        let _: () = msg_send![btn, setContentTintColor: Some(&*tint)];
    }
}

/// 创建复选框按钮 (`NSButtonType::Switch`，弱化色调)。
pub fn make_checkbox(
    mtm: MainThreadMarker,
    frame: NSRect,
    title: &str,
    checked: bool,
    tag: isize,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSButton> {
    unsafe {
        let btn = NSButton::initWithFrame(mtm.alloc(), frame);
        btn.setButtonType(NSButtonType::Switch);
        btn.setTitle(&NSString::from_str(title));
        btn.setFont(Some(&NSFont::systemFontOfSize(12.5)));
        let tint = pale_ink(0.55);
        let _: () = msg_send![&*btn, setContentTintColor: Some(&*tint)];
        btn.setState(if checked {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        btn.setTag(tag);
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn
    }
}

/// 创建下拉选择框 (`NSPopUpButton`)；`bordered = false` 时呈极简淡色语言选择条风格。
pub fn make_popup(
    mtm: MainThreadMarker,
    frame: NSRect,
    items: &[&str],
    selected_idx: usize,
    bordered: bool,
    tag: isize,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSPopUpButton> {
    unsafe {
        let popup = NSPopUpButton::initWithFrame_pullsDown(mtm.alloc(), frame, false);
        popup.setBordered(bordered);
        let font = NSFont::systemFontOfSize(12.5);
        popup.setFont(Some(&font));
        let tint = pale_ink(0.62);
        let _: () = msg_send![&*popup, setContentTintColor: Some(&*tint)];
        for item in items {
            popup.addItemWithTitle(&NSString::from_str(item));
        }
        if selected_idx < items.len() {
            popup.selectItemAtIndex(selected_idx as isize);
        }
        popup.setTag(tag);
        popup.setTarget(Some(target));
        popup.setAction(Some(action));
        popup
    }
}

/// 创建单行可编辑输入框 (`NSTextField`，柔和低对比背景)。
pub fn make_text_field(
    mtm: MainThreadMarker,
    frame: NSRect,
    value: &str,
    placeholder: &str,
) -> Retained<NSTextField> {
    let field = NSTextField::initWithFrame(mtm.alloc(), frame);
    field.setStringValue(&NSString::from_str(value));
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field.setEditable(true);
    field.setSelectable(true);
    field.setBezeled(false);
    field.setDrawsBackground(true);
    field.setBackgroundColor(Some(&pale_ink(0.045)));
    field.setTextColor(Some(&pale_ink(0.78)));
    field.setFont(Some(&NSFont::systemFontOfSize(12.5)));
    field.setWantsLayer(true);
    if let Some(layer) = field.layer() {
        layer.setCornerRadius(5.0);
    }
    field
}

/// 创建单行密码/密钥输入框 (`NSSecureTextField`，柔和低对比背景)。
pub fn make_secure_field(
    mtm: MainThreadMarker,
    frame: NSRect,
    placeholder: &str,
) -> Retained<NSSecureTextField> {
    let field = NSSecureTextField::initWithFrame(mtm.alloc(), frame);
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field.setEditable(true);
    field.setSelectable(true);
    field.setBezeled(false);
    field.setDrawsBackground(true);
    field.setBackgroundColor(Some(&pale_ink(0.045)));
    field.setTextColor(Some(&pale_ink(0.78)));
    field.setFont(Some(&NSFont::systemFontOfSize(12.5)));
    field.setWantsLayer(true);
    if let Some(layer) = field.layer() {
        layer.setCornerRadius(5.0);
    }
    field
}

/// 创建滑块控件 (`NSSlider`)。
pub fn make_slider(
    mtm: MainThreadMarker,
    frame: NSRect,
    min: f64,
    max: f64,
    value: f64,
    tag: isize,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSSlider> {
    unsafe {
        let slider = NSSlider::initWithFrame(mtm.alloc(), frame);
        slider.setMinValue(min);
        slider.setMaxValue(max);
        slider.setDoubleValue(value);
        slider.setContinuous(true);
        let tint = pale_ink(0.50);
        let _: () = msg_send![&*slider, setTrackFillColor: Some(&*tint)];
        slider.setTag(tag);
        slider.setTarget(Some(target));
        slider.setAction(Some(action));
        slider
    }
}

/// 为 `NSTextView` 应用舒适的段落行间距（同时更新 `defaultParagraphStyle`、`typingAttributes` 与现有 `textStorage`）。
pub fn apply_text_view_line_spacing(tv: &NSTextView, line_spacing: f64) {
    unsafe {
        let style: Retained<AnyObject> = msg_send![objc2::class!(NSMutableParagraphStyle), new];
        let _: () = msg_send![&*style, setLineSpacing: line_spacing as CGFloat];
        let _: () = msg_send![&*style, setParagraphSpacing: 3.0 as CGFloat];
        let _: () = msg_send![tv, setDefaultParagraphStyle: Some(&*style)];

        let attr_key = NSString::from_str("NSParagraphStyle");
        let typing_dict: Option<Retained<AnyObject>> = msg_send![tv, typingAttributes];
        if let Some(dict) = typing_dict {
            let mutable_dict: Retained<AnyObject> = msg_send![&*dict, mutableCopy];
            let _: () = msg_send![&*mutable_dict, setObject: &*style, forKey: &*attr_key];
            let _: () = msg_send![tv, setTypingAttributes: &*mutable_dict];
        }

        let len = tv.string().length();
        if len > 0 {
            let storage: Option<Retained<AnyObject>> = msg_send![tv, textStorage];
            if let Some(st) = storage {
                let range = NSRange::new(0, len);
                let _: () = msg_send![
                    &*st,
                    addAttribute: &*attr_key,
                    value: &*style,
                    range: range
                ];
            }
        }
    }
}

/// 创建无边框、通透扁平的多行 `NSTextView`（采用淡雅文字、柔和光标色与舒适行间距）。
pub fn make_scrollable_text_view(
    mtm: MainThreadMarker,
    frame: NSRect,
    editable: bool,
    font_size: f64,
    delegate: Option<&AnyObject>,
) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    unsafe {
        let scroll = NSScrollView::initWithFrame(mtm.alloc(), frame);
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setDrawsBackground(false);

        let content_size = scroll.contentSize();
        let text_frame = CGRect::new(CGPoint::new(0.0, 0.0), content_size);
        let text_view = NSTextView::initWithFrame(mtm.alloc(), text_frame);
        text_view.setEditable(editable);
        text_view.setSelectable(true);
        text_view.setRichText(false);
        text_view.setDrawsBackground(false);
        text_view.setFont(Some(&NSFont::systemFontOfSize(font_size as CGFloat)));
        text_view.setTextColor(Some(&pale_ink(0.78)));
        let insertion_color = pale_ink(0.55);
        text_view.setInsertionPointColor(Some(&insertion_color));
        text_view.setVerticallyResizable(true);
        text_view.setHorizontallyResizable(false);
        let inset = NSSize::new(12.0, 10.0);
        text_view.setTextContainerInset(inset);
        if let Some(container) = text_view.textContainer() {
            container.setWidthTracksTextView(true);
            container.setContainerSize(NSSize::new(content_size.width, 10_000_000.0));
        }
        if editable {
            let _: () = msg_send![&*text_view, setAutomaticQuoteSubstitutionEnabled: false];
            let _: () = msg_send![&*text_view, setAutomaticDashSubstitutionEnabled: false];
            let _: () = msg_send![&*text_view, setAutomaticSpellingCorrectionEnabled: false];
        }
        apply_text_view_line_spacing(&text_view, 5.5);
        if let Some(del) = delegate {
            let _: () = msg_send![&*text_view, setDelegate: Some(del)];
        }

        scroll.setDocumentView(Some(&text_view));
        (scroll, text_view)
    }
}

/// 读取 `NSTextView` 内的全部纯文本。
pub fn get_text_view_string(tv: &NSTextView) -> String {
    tv.string().to_string()
}

/// 设置 `NSTextView` 的纯文本与淡雅色调（区分占位符浅灰字与柔和正文颜色）。
pub fn set_text_view_styled(tv: &NSTextView, text: &str, is_muted: bool) {
    let s = NSString::from_str(text);
    tv.setString(&s);
    let color = if is_muted {
        pale_ink(0.28)
    } else {
        pale_ink(0.78)
    };
    tv.setTextColor(Some(&color));
    apply_text_view_line_spacing(tv, 5.5);
}

/// 设置 `NSTextView` 的纯文本内容（默认柔和正文色）。
pub fn set_text_view_string(tv: &NSTextView, text: &str) {
    set_text_view_styled(tv, text, false);
}

/// 向 `NSTextView` 末尾追加流式增量文本并自动滚动到底部。
pub fn append_text_view_string(tv: &NSTextView, delta: &str, scroll_to_end: bool) {
    let mut current = tv.string().to_string();
    current.push_str(delta);
    let s = NSString::from_str(&current);
    tv.setString(&s);
    tv.setTextColor(Some(&pale_ink(0.78)));
    apply_text_view_line_spacing(tv, 5.5);
    if scroll_to_end {
        let len = s.length();
        tv.scrollRangeToVisible(NSRange::new(len, 0));
    }
}

/// 读取 `NSTextField` 或 `NSSecureTextField` 文本。
pub fn get_field_string(field: &NSTextField) -> String {
    field.stringValue().to_string()
}

/// 设置 `NSTextField` 文本。
pub fn set_field_string(field: &NSTextField, value: &str) {
    let s = NSString::from_str(value);
    field.setStringValue(&s);
}

/// 将字符串写入 macOS 系统剪贴板。
pub fn copy_to_pasteboard(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let ns_str = NSString::from_str(text);
    let pb_type = unsafe { NSPasteboardTypeString };
    pb.setString_forType(&ns_str, pb_type)
}

/// 将子视图添加到父视图。
pub fn add_subview(parent: &NSView, child: &NSView) {
    parent.addSubview(child);
}

/// 将静态标签设置为右对齐（用于用户气泡上方“你”标签或百分比数值标签）。
pub fn set_label_alignment_right(label: &NSTextField) {
    label.setAlignment(NSTextAlignment::Right);
}

/// 创建自动换行的静态说明标签（用于设置页底部的多行说明文字，随宽度自适应折行）。
pub fn make_wrapping_label(
    mtm: MainThreadMarker,
    frame: NSRect,
    text: &str,
    font_size: f64,
    secondary: bool,
) -> Retained<NSTextField> {
    let label = make_label(mtm, frame, text, font_size, false, secondary);
    unsafe {
        let _: () = msg_send![&*label, setUsesSingleLineMode: Bool::NO];
        let _: () = msg_send![&*label, setPreferredMaxLayoutWidth: frame.size.width];
        if let Some(cell) = label.cell() {
            let _: () = msg_send![&*cell, setWraps: Bool::YES];
            let _: () = msg_send![&*cell, setScrollable: Bool::NO];
            let _: () = msg_send![&*cell, setLineBreakMode: 0usize];
        }
    }
    label
}

/// 更新自动换行标签的尺寸与 `preferredMaxLayoutWidth`。
pub fn set_wrapping_label_frame(label: &NSTextField, frame: NSRect) {
    label.setFrame(frame);
    unsafe {
        let _: () = msg_send![label, setPreferredMaxLayoutWidth: frame.size.width];
    }
}

/// 清理 AI 回复中的常见 Markdown 标记（如 `**粗体**`、`### 标题`），让纯文本气泡干净易读。
pub fn clean_markdown_for_bubble(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed_start = line.trim_start();
        let without_heading = if let Some(rest) = trimmed_start.strip_prefix("#### ") {
            rest
        } else if let Some(rest) = trimmed_start.strip_prefix("### ") {
            rest
        } else if let Some(rest) = trimmed_start.strip_prefix("## ") {
            rest
        } else if let Some(rest) = trimmed_start.strip_prefix("# ") {
            rest
        } else {
            line
        };
        let cleaned = without_heading.replace("**", "").replace("__", "");
        lines.push(cleaned);
    }
    if raw.ends_with('\n') {
        let mut joined = lines.join("\n");
        joined.push('\n');
        joined
    } else {
        lines.join("\n")
    }
}

/// 创建可复制、自动换行的聊天气泡文本标签，并返回 `(标签实例, 实测文字包裹尺寸)`。
pub fn make_wrapping_message_label(
    mtm: MainThreadMarker,
    text: &str,
    max_width: f64,
    font_size: f64,
    is_muted: bool,
) -> (Retained<NSTextField>, NSSize) {
    let safe_max_w = max_width.max(80.0);
    let label = NSTextField::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(safe_max_w, 22.0)),
    );
    label.setStringValue(&NSString::from_str(text));
    label.setBezeled(false);
    label.setDrawsBackground(false);
    label.setEditable(false);
    label.setSelectable(true);
    label.setFont(Some(&NSFont::systemFontOfSize(font_size as CGFloat)));
    let color = if is_muted {
        pale_ink(0.36)
    } else {
        pale_ink(0.80)
    };
    label.setTextColor(Some(&color));

    let measured = unsafe {
        let _: () = msg_send![&*label, setUsesSingleLineMode: Bool::NO];
        let _: () = msg_send![&*label, setPreferredMaxLayoutWidth: safe_max_w as CGFloat];
        if let Some(cell) = label.cell() {
            let _: () = msg_send![&*cell, setWraps: Bool::YES];
            let _: () = msg_send![&*cell, setScrollable: Bool::NO];
            // NSLineBreakByWordWrapping = 0
            let _: () = msg_send![&*cell, setLineBreakMode: 0usize];
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(safe_max_w, 20_000.0));
            let sz: NSSize = msg_send![&*cell, cellSizeForBounds: bounds];
            NSSize::new(
                (sz.width.ceil() + 2.0).clamp(18.0, safe_max_w),
                (sz.height.ceil() + 2.0).max(18.0),
            )
        } else {
            NSSize::new(safe_max_w, 20.0)
        }
    };
    (label, measured)
}

/// 带柔和圆角底框的多行可滚动文本输入框（用于设置页的“翻译提示词”与“系统提示词”等大段文本配置）。
pub struct MultilineInputWidget {
    pub surface: Retained<NSBox>,
    pub scroll: Retained<NSScrollView>,
    pub tv: Retained<NSTextView>,
}

/// 创建带柔和淡色圆角边框的多行可滚动编辑框。
pub fn make_multiline_input(
    mtm: MainThreadMarker,
    frame: NSRect,
    value: &str,
    font_size: f64,
) -> MultilineInputWidget {
    let surface = NSBox::initWithFrame(mtm.alloc(), frame);
    surface.setBoxType(NSBoxType::Custom);
    surface.setBorderWidth(0.5);
    surface.setBorderColor(&pale_ink(0.08));
    surface.setCornerRadius(6.0);
    surface.setFillColor(&pale_ink(0.045));
    surface.setContentViewMargins(CGSize::new(0.0, 0.0));

    let inner_frame = NSRect::new(
        NSPoint::new(frame.origin.x + 2.0, frame.origin.y + 2.0),
        NSSize::new(
            (frame.size.width - 4.0).max(20.0),
            (frame.size.height - 4.0).max(20.0),
        ),
    );
    let (scroll, tv) = make_scrollable_text_view(mtm, inner_frame, true, font_size, None);
    tv.setTextContainerInset(NSSize::new(8.0, 7.0));
    set_text_view_string(&tv, value);

    MultilineInputWidget {
        surface,
        scroll,
        tv,
    }
}

/// 更新多行编辑框的位置与宽高（自适应窗口缩放）。
pub fn set_multiline_input_frame(widget: &MultilineInputWidget, frame: NSRect) {
    widget.surface.setFrame(frame);
    let inner_frame = NSRect::new(
        NSPoint::new(frame.origin.x + 2.0, frame.origin.y + 2.0),
        NSSize::new(
            (frame.size.width - 4.0).max(20.0),
            (frame.size.height - 4.0).max(20.0),
        ),
    );
    widget.scroll.setFrame(inner_frame);
}

/// 隐藏或显示多行编辑框。
pub fn set_multiline_input_hidden(widget: &MultilineInputWidget, hidden: bool) {
    widget.surface.setHidden(hidden);
    widget.scroll.setHidden(hidden);
}
