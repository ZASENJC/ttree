//! Native shortcut fields receive key events directly, without the text field editor.
use std::cell::RefCell;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSFont, NSTextField, NSWindow};
use objc2_foundation::{NSObjectProtocol, NSRect, NSString};
use ttree_platform::shortcut_recorder::{CaptureResult, ShortcutRecorder};

use crate::ui_helpers::pale_ink;

pub struct ShortcutFieldIvars {
    recorder: RefCell<ShortcutRecorder>,
}

define_class!(
    #[unsafe(super(NSTextField))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ShortcutFieldIvars]
    #[name = "TTreeShortcutField"]
    pub struct ShortcutField;

    unsafe impl NSObjectProtocol for ShortcutField {}

    impl ShortcutField {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> Bool { Bool::YES }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> Bool {
            self.ivars().recorder.borrow_mut().begin();
            self.setBackgroundColor(Some(&pale_ink(0.10)));
            crate::app::on_shortcut_recording(true);
            Bool::YES
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> Bool {
            self.setBackgroundColor(Some(&pale_ink(0.045)));
            crate::app::on_shortcut_recording(false);
            Bool::YES
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) { self.record(event); }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> Bool {
            let focused = self.window().and_then(|window| window.firstResponder())
                .is_some_and(|responder| responder.isEqual(Some(self)));
            if focused {
                self.record(event);
                Bool::YES
            } else {
                Bool::NO
            }
        }
    }
);

impl ShortcutField {
    pub fn new(mtm: MainThreadMarker, frame: NSRect, value: &str) -> Retained<Self> {
        let allocated = mtm.alloc::<Self>().set_ivars(ShortcutFieldIvars {
            recorder: RefCell::new(ShortcutRecorder::default()),
        });
        let field: Retained<Self> = unsafe { msg_send![super(allocated), initWithFrame: frame] };
        field.setEditable(false);
        field.setSelectable(false);
        field.setBezeled(false);
        field.setDrawsBackground(true);
        field.setBackgroundColor(Some(&pale_ink(0.045)));
        field.setTextColor(Some(&pale_ink(0.78)));
        field.setFont(Some(&NSFont::systemFontOfSize(12.5)));
        field.setPlaceholderString(Some(&NSString::from_str("点击后按组合键")));
        field.setWantsLayer(true);
        if let Some(layer) = field.layer() {
            layer.setCornerRadius(5.0);
        }
        field.set_value(value);
        field
    }

    pub fn set_value(&self, value: &str) {
        let display = {
            let mut recorder = self.ivars().recorder.borrow_mut();
            recorder.set(value);
            recorder.display()
        };
        self.setStringValue(&NSString::from_str(&display));
    }

    pub fn value(&self) -> String {
        self.ivars().recorder.borrow().value().to_owned()
    }

    fn record(&self, event: &NSEvent) {
        let Some(key) = key(event.keyCode()) else {
            return;
        };
        let flags = event.modifierFlags();
        let mut modifiers = Modifiers::empty();
        for (flag, modifier) in [
            (NSEventModifierFlags::Control, Modifiers::CONTROL),
            (NSEventModifierFlags::Option, Modifiers::ALT),
            (NSEventModifierFlags::Shift, Modifiers::SHIFT),
            (NSEventModifierFlags::Command, Modifiers::SUPER),
        ] {
            if flags.contains(flag) {
                modifiers |= modifier;
            }
        }
        let (result, display) = {
            let mut recorder = self.ivars().recorder.borrow_mut();
            let result = recorder.capture(key, modifiers);
            (result, recorder.display())
        };
        self.setStringValue(&NSString::from_str(&display));
        if let Some(window) = self.window() {
            match result {
                CaptureResult::Cancelled => {
                    window.makeFirstResponder(None);
                }
                CaptureResult::Navigate { backwards: true } => {
                    window.selectPreviousKeyView(Some(self))
                }
                CaptureResult::Navigate { backwards: false } => {
                    window.selectNextKeyView(Some(self))
                }
                _ => {}
            }
        }
    }
}

pub fn is_recording(window: &NSWindow) -> bool {
    window.isKeyWindow()
        && window.firstResponder().is_some_and(|responder| {
            responder
                .downcast_ref::<ShortcutField>()
                .is_some_and(|field| !field.isHiddenOrHasHiddenAncestor())
        })
}

fn key(code: u16) -> Option<Code> {
    let token = match code {
        0 => "A",
        1 => "S",
        2 => "D",
        3 => "F",
        4 => "H",
        5 => "G",
        6 => "Z",
        7 => "X",
        8 => "C",
        9 => "V",
        11 => "B",
        12 => "Q",
        13 => "W",
        14 => "E",
        15 => "R",
        16 => "Y",
        17 => "T",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        22 => "6",
        23 => "5",
        24 => "Equal",
        25 => "9",
        26 => "7",
        27 => "Minus",
        28 => "8",
        29 => "0",
        30 => "BracketRight",
        31 => "O",
        32 => "U",
        33 => "BracketLeft",
        34 => "I",
        35 => "P",
        36 => "Enter",
        37 => "L",
        38 => "J",
        39 => "Quote",
        40 => "K",
        41 => "Semicolon",
        42 => "Backslash",
        43 => "Comma",
        44 => "Slash",
        45 => "N",
        46 => "M",
        47 => "Period",
        48 => "Tab",
        49 => "Space",
        50 => "Backquote",
        51 => "Backspace",
        53 => "Escape",
        65 => "NumpadDecimal",
        67 => "NumpadMultiply",
        69 => "NumpadAdd",
        75 => "NumpadDivide",
        76 => "NumpadEnter",
        78 => "NumpadSubtract",
        81 => "NumpadEqual",
        82 => "Numpad0",
        83 => "Numpad1",
        84 => "Numpad2",
        85 => "Numpad3",
        86 => "Numpad4",
        87 => "Numpad5",
        88 => "Numpad6",
        89 => "Numpad7",
        91 => "Numpad8",
        92 => "Numpad9",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        105 => "F13",
        107 => "F14",
        113 => "F15",
        106 => "F16",
        64 => "F17",
        79 => "F18",
        80 => "F19",
        90 => "F20",
        115 => "Home",
        116 => "PageUp",
        117 => "Delete",
        119 => "End",
        121 => "PageDown",
        123 => "Left",
        124 => "Right",
        125 => "Down",
        126 => "Up",
        _ => return None,
    };
    token.parse::<HotKey>().ok().map(|hotkey| hotkey.key)
}
