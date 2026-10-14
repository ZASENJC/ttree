//! `ttree` —— 100% 纯 Rust 原生 macOS AI 翻译与 OCR 桌面端入口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
mod state;
#[cfg(target_os = "macos")]
mod ui_helpers;

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("TTREE 纯 Rust 原生 AppKit 客户端仅支持 macOS 运行。");
}

#[cfg(target_os = "macos")]
fn main() {
    macos_app::run();
}

#[cfg(target_os = "macos")]
mod macos_app {
    use std::cell::RefCell;
    use std::sync::Arc;

    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, Sel};
    use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEvent,
        NSEventModifierFlags, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSTextView,
        NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
        NSVisualEffectView, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility,
    };
    use objc2_foundation::{
        NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize,
        NSString, NSTimer,
    };
    use tokio::sync::mpsc::{self, UnboundedReceiver};

    use ttree_core::{WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH};
    use ttree_storage::{HistoryStore, SettingsStore};

    use crate::state::{ActionTag, AppController, TabMode, UiEvent};

    struct GlobalUiContext {
        ctrl: AppController,
        ui_rx: UnboundedReceiver<UiEvent>,
        _status_item: Retained<NSStatusItem>,
    }

    thread_local! {
        static UI_CTX: RefCell<Option<GlobalUiContext>> = const { RefCell::new(None) };
    }

    fn with_ctx<F: FnOnce(&mut GlobalUiContext)>(f: F) {
        UI_CTX.with(|cell| {
            if let Ok(mut borrow) = cell.try_borrow_mut() {
                if let Some(ctx) = borrow.as_mut() {
                    f(ctx);
                }
            }
        });
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "TTreeActionDispatcher"]
        pub struct ActionDispatcher;

        unsafe impl NSObjectProtocol for ActionDispatcher {}

        impl ActionDispatcher {
            #[unsafe(method(handleControl:))]
            fn handle_control(&self, sender: &AnyObject) {
                let raw_tag: isize = unsafe { msg_send![sender, tag] };
                if let Some(tag) = ActionTag::from_raw(raw_tag) {
                    with_ctx(|ctx| ctx.ctrl.on_action(tag));
                }
            }

            #[unsafe(method(handleTimer:))]
            fn handle_timer(&self, _timer: &AnyObject) {
                with_ctx(|ctx| {
                    while let Ok(hk_ev) = GlobalHotKeyEvent::receiver().try_recv() {
                        if hk_ev.state == HotKeyState::Pressed {
                            if let Some(action) = ctx.ctrl.parsed_shortcuts.action_for_id(hk_ev.id)
                            {
                                ctx.ctrl.handle_hotkey_action(action);
                            }
                        }
                    }
                    while let Ok(ui_ev) = ctx.ui_rx.try_recv() {
                        ctx.ctrl.handle_ui_event(ui_ev);
                    }
                    ctx.ctrl.poll_tick();
                });
            }

            #[unsafe(method(textView:doCommandBySelector:))]
            fn text_view_do_command(&self, _text_view: &NSTextView, command: Sel) -> Bool {
                if command == sel!(cancelOperation:) {
                    with_ctx(|ctx| ctx.ctrl.on_action(ActionTag::HideOrEscape));
                    return Bool::YES;
                }
                if command == sel!(insertNewline:) {
                    let flags = NSEvent::modifierFlags_class();
                    if flags.contains(NSEventModifierFlags::Shift) {
                        return Bool::NO;
                    }
                    with_ctx(|ctx| match ctx.ctrl.active_tab {
                        TabMode::Translate => ctx.ctrl.start_translation(),
                        TabMode::Chat => ctx.ctrl.send_chat_from_input(),
                        TabMode::Settings => {}
                    });
                    return Bool::YES;
                }
                Bool::NO
            }

            #[unsafe(method(windowDidResignKey:))]
            fn window_did_resign_key(&self, _notification: &AnyObject) {
                with_ctx(|ctx| ctx.ctrl.on_window_did_resign_key());
            }

            #[unsafe(method(windowDidResize:))]
            fn window_did_resize(&self, _notification: &AnyObject) {
                with_ctx(|ctx| ctx.ctrl.on_window_did_resize());
            }

            #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
            fn app_should_handle_reopen(&self, _app: &AnyObject, _has_visible: Bool) -> Bool {
                with_ctx(|ctx| {
                    ctx.ctrl.switch_tab(TabMode::Translate);
                    ctx.ctrl.show_and_focus_window();
                });
                Bool::YES
            }
        }
    );

    impl ActionDispatcher {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            unsafe { msg_send![mtm.alloc::<Self>(), init] }
        }
    }

    pub fn run() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("初始化 Tokio 异步运行时失败");

        let mtm = MainThreadMarker::new().expect("必须在 macOS 主线程启动 AppKit UI");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        let dispatcher = ActionDispatcher::new(mtm);
        unsafe {
            let _: () = msg_send![&*app, setDelegate: Some(&*dispatcher)];
        }

        install_standard_main_menu(mtm, &app);

        let settings = Arc::new(SettingsStore::open_default());
        let history = Arc::new(HistoryStore::open_default());
        let hotkey_mgr = GlobalHotKeyManager::new().expect("初始化全局快捷键管理器失败");

        let (win_w, win_h) = settings.window_size();
        let appearance = settings.appearance();
        let (window, effect_view) =
            build_main_window(mtm, win_w, win_h, appearance.panel_opacity, &dispatcher);

        let status_item = build_status_bar_item(mtm, &dispatcher);
        let (ui_tx, ui_rx) = mpsc::unbounded_channel();

        let mut ctrl = AppController::build(
            mtm,
            rt.handle().clone(),
            ui_tx,
            settings,
            history,
            hotkey_mgr,
            window,
            effect_view,
            &dispatcher,
        );

        if std::env::args().any(|a| a == "--show") {
            ctrl.show_and_focus_window();
        }

        UI_CTX.with(|cell| {
            *cell.borrow_mut() = Some(GlobalUiContext {
                ctrl,
                ui_rx,
                _status_item: status_item,
            });
        });

        unsafe {
            let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                0.025,
                &dispatcher,
                sel!(handleTimer:),
                None,
                true,
            );
            let run_loop = NSRunLoop::mainRunLoop();
            run_loop.addTimer_forMode(&timer, NSRunLoopCommonModes);
        }

        app.run();
    }

    fn build_main_window(
        mtm: MainThreadMarker,
        width: f64,
        height: f64,
        opacity: f64,
        dispatcher: &ActionDispatcher,
    ) -> (Retained<NSWindow>, Retained<NSVisualEffectView>) {
        unsafe {
            let frame = NSRect::new(NSPoint::new(240.0, 240.0), NSSize::new(width, height));
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView;
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setTitle(&NSString::from_str("TTREE"));
            window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            window.setTitlebarAppearsTransparent(true);
            window.setMovableByWindowBackground(true);
            window.setReleasedWhenClosed(false);
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
            window.setAlphaValue(opacity);
            window.setMinSize(NSSize::new(WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT));

            for btn_kind in [
                NSWindowButton::CloseButton,
                NSWindowButton::MiniaturizeButton,
                NSWindowButton::ZoomButton,
            ] {
                if let Some(btn) = window.standardWindowButton(btn_kind) {
                    btn.setHidden(true);
                }
            }

            let _: () = msg_send![&*window, setDelegate: Some(dispatcher)];

            let effect_view = NSVisualEffectView::initWithFrame(mtm.alloc(), frame);
            effect_view.setMaterial(NSVisualEffectMaterial::Popover);
            effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect_view.setState(NSVisualEffectState::Active);

            window.setContentView(Some(&effect_view));
            (window, effect_view)
        }
    }

    fn build_status_bar_item(
        mtm: MainThreadMarker,
        dispatcher: &ActionDispatcher,
    ) -> Retained<NSStatusItem> {
        unsafe {
            let status_bar = NSStatusBar::systemStatusBar();
            let item = status_bar.statusItemWithLength(-1.0);
            if let Some(btn) = item.button(mtm) {
                btn.setTitle(&NSString::from_str("译"));
            }

            let menu = NSMenu::new(mtm);
            let action_sel = sel!(handleControl:);

            let add_item = |title: &str, tag: ActionTag, key: &str| {
                let mi = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &NSString::from_str(title),
                    Some(action_sel),
                    &NSString::from_str(key),
                );
                mi.setTag(tag as isize);
                mi.setTarget(Some(dispatcher));
                menu.addItem(&mi);
            };

            add_item("翻译窗口", ActionTag::TabTranslate, "");
            add_item("截图 OCR 翻译", ActionTag::TriggerOcr, "");
            add_item("AI 对话", ActionTag::TabChat, "");
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            add_item("偏好设置…", ActionTag::TabSettings, ",");
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            add_item("退出 TTREE", ActionTag::TrayQuit, "q");

            // 点击菜单项时如果窗口尚未显示则自动唤起（除了截图 OCR 与退出）
            item.setMenu(Some(&menu));
            item
        }
    }

    /// 为 Accessory 模式应用安装标准 Edit 菜单，使 NSTextView / NSTextField 原生支持 Cmd+C / V / X / A / Z。
    fn install_standard_main_menu(mtm: MainThreadMarker, app: &NSApplication) {
        unsafe {
            let main_menu = NSMenu::new(mtm);

            // 1. App 子菜单
            let app_item = NSMenuItem::new(mtm);
            let app_menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("TTREE"));
            let close_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str("隐藏窗口"),
                Some(sel!(orderOut:)),
                &NSString::from_str("w"),
            );
            let quit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str("退出 TTREE"),
                Some(sel!(terminate:)),
                &NSString::from_str("q"),
            );
            app_menu.addItem(&close_item);
            app_menu.addItem(&NSMenuItem::separatorItem(mtm));
            app_menu.addItem(&quit_item);
            app_item.setSubmenu(Some(&app_menu));
            main_menu.addItem(&app_item);

            // 2. Edit 子菜单 (支持 Cmd+Z / Cmd+X / Cmd+C / Cmd+V / Cmd+A)
            let edit_item = NSMenuItem::new(mtm);
            let edit_menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("编辑"));

            let edit_entries: [(&str, Sel, &str); 5] = [
                ("撤销", sel!(undo:), "z"),
                ("剪切", sel!(cut:), "x"),
                ("复制", sel!(copy:), "c"),
                ("粘贴", sel!(paste:), "v"),
                ("全选", sel!(selectAll:), "a"),
            ];
            for (title, action, key) in edit_entries {
                let mi = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &NSString::from_str(title),
                    Some(action),
                    &NSString::from_str(key),
                );
                edit_menu.addItem(&mi);
            }
            edit_item.setSubmenu(Some(&edit_menu));
            main_menu.addItem(&edit_item);

            app.setMainMenu(Some(&main_menu));
        }
    }
}
