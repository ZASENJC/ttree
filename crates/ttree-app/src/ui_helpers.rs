//! macOS 原生 AppKit (`objc2-app-kit`) 扁平化控件与上下分栏排版辅助函数。

#![cfg(target_os = "macos")]
#![allow(clippy::too_many_arguments)]

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{msg_send, ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSBezelStyle, NSBox, NSBoxType, NSButton, NSButtonType, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSImage, NSPasteboard, NSPasteboardTypeString, NSPopUpButton,
    NSScrollView, NSSecureTextField, NSSlider, NSTextField, NSTextView, NSView,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

/// 将以顶部为原点的 `(x, top_y, width, height)` 转换为 AppKit 默认左下角原点的 `NSRect`。
pub fn top_rect(parent_height: f64, x: f64, top_y: f64, width: f64, height: f64) -> NSRect {
    let y = (parent_height - top_y - height).max(0.0);
    NSRect::new(NSPoint::new(x.max(0.0), y), NSSize::new(width.max(1.0), height.max(1.0)))
}

/// 创建静态文本标签 (`NSTextField` 只读无边框)。
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
        NSColor::secondaryLabelColor()
    } else {
        NSColor::labelColor()
    };
    label.setTextColor(Some(&color));
    label
}

/// 创建 1px 极淡分隔线（用于上下分栏与左右侧栏分隔）。
pub fn make_divider(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSBox> {
    let line = NSBox::initWithFrame(mtm.alloc(), frame);
    line.setBoxType(NSBoxType::Custom);
    line.setBorderWidth(0.0);
    line.setCornerRadius(0.0);
    line.setFillColor(&NSColor::separatorColor());
    line.setContentViewMargins(CGSize::new(0.0, 0.0));
    line
}

/// 创建扁平色块背景容器 (`NSBox` 无边框)。
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
    surface.setFillColor(&NSColor::controlBackgroundColor().colorWithAlphaComponent(alpha as CGFloat));
    surface.setContentViewMargins(CGSize::new(0.0, 0.0));
    surface
}

/// 创建圆角按钮 (`NSButton`)。
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
        btn.setTitle(&NSString::from_str(title));
        #[allow(deprecated)]
        btn.setBezelStyle(NSBezelStyle::Rounded);
        btn.setFont(Some(&NSFont::systemFontOfSize(12.5)));
        btn.setTag(tag);
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn
    }
}

/// 创建无边框扁平文字按钮（用于「复制」「完成」、顶部模式切换、设置左侧导航）。
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
            NSColor::controlAccentColor()
        } else {
            NSColor::secondaryLabelColor()
        };
        let _: () = msg_send![&*btn, setContentTintColor: Some(&*color)];
        btn.setTag(tag);
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn
    }
}

/// 创建右上角功能区 SF Symbols 图标按钮（如 `pin`, `gearshape`, `square.and.pencil`）。
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

/// 更新图标按钮的 SF Symbol 与激活态颜色。
pub fn set_button_symbol(
    btn: &NSButton,
    symbol_name: &str,
    fallback_title: &str,
    active: bool,
) {
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
            NSColor::controlAccentColor()
        } else {
            NSColor::secondaryLabelColor()
        };
        let _: () = msg_send![btn, setContentTintColor: Some(&*tint)];
    }
}

/// 设置扁平按钮的文字颜色（用于激活态切换）。
pub fn set_flat_button_active(btn: &NSButton, active: bool, is_accent: bool) {
    unsafe {
        let tint = if active {
            if is_accent {
                NSColor::controlAccentColor()
            } else {
                NSColor::labelColor()
            }
        } else {
            NSColor::secondaryLabelColor()
        };
        let _: () = msg_send![btn, setContentTintColor: Some(&*tint)];
    }
}

/// 创建复选框按钮 (`NSButtonType::Switch`)。
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
        btn.setFont(Some(&NSFont::systemFontOfSize(13.0)));
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

/// 创建下拉选择框 (`NSPopUpButton`)；`bordered = false` 时呈原版扁平语言选择条风格。
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
        let font = if bordered {
            NSFont::systemFontOfSize(12.5)
        } else {
            NSFont::boldSystemFontOfSize(13.0)
        };
        popup.setFont(Some(&font));
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

/// 创建单行可编辑输入框 (`NSTextField`)。
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
    field.setBezeled(true);
    field.setDrawsBackground(true);
    field.setFont(Some(&NSFont::systemFontOfSize(12.5)));
    field
}

/// 创建单行密码/密钥输入框 (`NSSecureTextField`)。
pub fn make_secure_field(
    mtm: MainThreadMarker,
    frame: NSRect,
    placeholder: &str,
) -> Retained<NSSecureTextField> {
    let field = NSSecureTextField::initWithFrame(mtm.alloc(), frame);
    field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    field.setEditable(true);
    field.setSelectable(true);
    field.setBezeled(true);
    field.setDrawsBackground(true);
    field.setFont(Some(&NSFont::systemFontOfSize(12.5)));
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
        slider.setTag(tag);
        slider.setTarget(Some(target));
        slider.setAction(Some(action));
        slider
    }
}

/// 创建无边框、通透扁平的多行 `NSTextView`（完全贴合原版上下分栏输入框与译文框视觉）。
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
        text_view.setTextColor(Some(&NSColor::labelColor()));
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

/// 设置 `NSTextView` 的纯文本与色调（区分占位符灰字与正文颜色）。
pub fn set_text_view_styled(tv: &NSTextView, text: &str, is_muted: bool) {
    let s = NSString::from_str(text);
    tv.setString(&s);
    let color = if is_muted {
        NSColor::tertiaryLabelColor()
    } else {
        NSColor::labelColor()
    };
    tv.setTextColor(Some(&color));
}

/// 设置 `NSTextView` 的纯文本内容（默认正文色）。
pub fn set_text_view_string(tv: &NSTextView, text: &str) {
    set_text_view_styled(tv, text, false);
}

/// 向 `NSTextView` 末尾追加流式增量文本并自动滚动到底部。
pub fn append_text_view_string(tv: &NSTextView, delta: &str, scroll_to_end: bool) {
    let mut current = tv.string().to_string();
    current.push_str(delta);
    let s = NSString::from_str(&current);
    tv.setString(&s);
    tv.setTextColor(Some(&NSColor::labelColor()));
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
