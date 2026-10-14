//! `ttree-app` 应用核心控制器、事件总线与三大视图（翻译 / AI 对话 / 设置）状态机。

#![cfg(target_os = "macos")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSBox, NSButton, NSControlStateValueOn, NSFloatingWindowLevel,
    NSModalResponseOK, NSNormalWindowLevel, NSOpenPanel, NSPopUpButton, NSScreen, NSScrollView,
    NSSecureTextField, NSSlider, NSTextField, NSTextView, NSView, NSVisualEffectView, NSWindow,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::UnboundedSender;

use ttree_core::{
    recent_chat_context, resolve_language_pair, AiBackendKind, AiTranslationConfig,
    AppearanceConfig, CancelSlot, ChatAiConfig, ChatMessage, Conversation, ShortcutConfig,
    ADAPTIVE_TARGET, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN, SOURCE_LANGUAGES, TARGET_LANGUAGES,
    WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
use ttree_ocr::OcrOutcome;
use ttree_platform_macos::hotkey::{parse_shortcut_config, ParsedShortcuts, ShortcutAction};
use ttree_storage::{HistoryStore, SettingsStore};

use crate::ui_helpers::{
    add_subview, append_text_view_string, copy_to_pasteboard, get_field_string,
    get_text_view_string, make_button, make_card_box, make_checkbox, make_label, make_popup,
    make_scrollable_text_view, make_secure_field, make_slider, make_text_field, set_field_string,
    set_text_view_string, top_rect,
};

const AUTO_TRANSLATE_DEBOUNCE: Duration = Duration::from_millis(420);
const RESIZE_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabMode {
    Translate,
    Chat,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(isize)]
pub enum ActionTag {
    TabTranslate = 1,
    TabChat = 2,
    TabSettings = 3,
    TriggerOcr = 4,
    TogglePin = 5,
    TrayQuit = 6,
    HideOrEscape = 7,

    TranslateNow = 10,
    ClearTranslate = 11,
    SwapLanguages = 12,
    SourceLangChanged = 13,
    TargetLangChanged = 14,
    CopyWebResult = 15,
    CopyAiResult = 16,

    SendChat = 20,
    NewConversation = 21,
    ClearChatHistory = 22,
    StopChatStream = 23,
    SelectConversation = 24,
    CopyLastReply = 25,

    SaveSettings = 30,
    AiTransBackendChanged = 31,
    AiTransFillDefaultUrl = 32,
    AiTransDiscoverModels = 33,
    AiTransModelPopupSelected = 34,
    AiTransBrowseGguf = 35,
    AiTransBrowseTokenizer = 36,
    AiTransClearKey = 37,

    ChatBackendChanged = 41,
    ChatFillDefaultUrl = 42,
    ChatDiscoverModels = 43,
    ChatModelPopupSelected = 44,
    ChatBrowseGguf = 45,
    ChatCopyFromTrans = 46,
    ChatClearKey = 47,

    ResetDefaultShortcuts = 50,
    OpacitySliderChanged = 51,
}

impl ActionTag {
    pub fn from_raw(tag: isize) -> Option<Self> {
        match tag {
            1 => Some(Self::TabTranslate),
            2 => Some(Self::TabChat),
            3 => Some(Self::TabSettings),
            4 => Some(Self::TriggerOcr),
            5 => Some(Self::TogglePin),
            6 => Some(Self::TrayQuit),
            7 => Some(Self::HideOrEscape),
            10 => Some(Self::TranslateNow),
            11 => Some(Self::ClearTranslate),
            12 => Some(Self::SwapLanguages),
            13 => Some(Self::SourceLangChanged),
            14 => Some(Self::TargetLangChanged),
            15 => Some(Self::CopyWebResult),
            16 => Some(Self::CopyAiResult),
            20 => Some(Self::SendChat),
            21 => Some(Self::NewConversation),
            22 => Some(Self::ClearChatHistory),
            23 => Some(Self::StopChatStream),
            24 => Some(Self::SelectConversation),
            25 => Some(Self::CopyLastReply),
            30 => Some(Self::SaveSettings),
            31 => Some(Self::AiTransBackendChanged),
            32 => Some(Self::AiTransFillDefaultUrl),
            33 => Some(Self::AiTransDiscoverModels),
            34 => Some(Self::AiTransModelPopupSelected),
            35 => Some(Self::AiTransBrowseGguf),
            36 => Some(Self::AiTransBrowseTokenizer),
            37 => Some(Self::AiTransClearKey),
            41 => Some(Self::ChatBackendChanged),
            42 => Some(Self::ChatFillDefaultUrl),
            43 => Some(Self::ChatDiscoverModels),
            44 => Some(Self::ChatModelPopupSelected),
            45 => Some(Self::ChatBrowseGguf),
            46 => Some(Self::ChatCopyFromTrans),
            47 => Some(Self::ChatClearKey),
            50 => Some(Self::ResetDefaultShortcuts),
            51 => Some(Self::OpacitySliderChanged),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum UiEvent {
    WebTranslateResult {
        gen: u64,
        result: Result<String, String>,
    },
    AiTranslateDelta {
        gen: u64,
        delta: String,
    },
    AiTranslateFinished {
        gen: u64,
        error: Option<String>,
    },
    OcrCompleted {
        outcome: Result<OcrOutcome, String>,
    },
    SelectionCapturedForTranslate {
        text: Option<String>,
        fallback_toggle: bool,
    },
    SelectionCapturedForChat {
        text: Option<String>,
        fallback_toggle: bool,
    },
    ChatDelta {
        gen: u64,
        delta: String,
    },
    ChatFinished {
        gen: u64,
        error: Option<String>,
    },
    ModelsDiscovered {
        for_chat: bool,
        result: Result<Vec<String>, String>,
    },
}

pub struct TranslateViewWidgets {
    pub root: Retained<NSView>,
    pub source_popup: Retained<NSPopUpButton>,
    pub swap_btn: Retained<NSButton>,
    pub target_popup: Retained<NSPopUpButton>,
    pub translate_btn: Retained<NSButton>,
    pub clear_btn: Retained<NSButton>,
    pub input_scroll: Retained<NSScrollView>,
    pub input_tv: Retained<NSTextView>,
    pub web_card: Retained<NSBox>,
    pub web_header_label: Retained<NSTextField>,
    pub web_copy_btn: Retained<NSButton>,
    pub web_scroll: Retained<NSScrollView>,
    pub web_tv: Retained<NSTextView>,
    pub ai_card: Retained<NSBox>,
    pub ai_header_label: Retained<NSTextField>,
    pub ai_copy_btn: Retained<NSButton>,
    pub ai_scroll: Retained<NSScrollView>,
    pub ai_tv: Retained<NSTextView>,
}

pub struct ChatViewWidgets {
    pub root: Retained<NSView>,
    pub conv_popup: Retained<NSPopUpButton>,
    pub new_conv_btn: Retained<NSButton>,
    pub clear_hist_btn: Retained<NSButton>,
    pub stop_btn: Retained<NSButton>,
    pub copy_reply_btn: Retained<NSButton>,
    pub transcript_scroll: Retained<NSScrollView>,
    pub transcript_tv: Retained<NSTextView>,
    pub input_scroll: Retained<NSScrollView>,
    pub input_tv: Retained<NSTextView>,
    pub send_btn: Retained<NSButton>,
}

pub struct SettingsViewWidgets {
    pub root: Retained<NSView>,
    pub ai_enabled_chk: Retained<NSButton>,
    pub ai_backend_popup: Retained<NSPopUpButton>,
    pub ai_base_url_field: Retained<NSTextField>,
    pub ai_key_field: Retained<NSSecureTextField>,
    pub ai_key_status_label: Retained<NSTextField>,
    pub ai_model_field: Retained<NSTextField>,
    pub ai_discovered_popup: Retained<NSPopUpButton>,
    pub ai_gguf_field: Retained<NSTextField>,
    pub ai_tokenizer_field: Retained<NSTextField>,
    pub ai_prompt_field: Retained<NSTextField>,

    pub chat_backend_popup: Retained<NSPopUpButton>,
    pub chat_base_url_field: Retained<NSTextField>,
    pub chat_key_field: Retained<NSSecureTextField>,
    pub chat_key_status_label: Retained<NSTextField>,
    pub chat_model_field: Retained<NSTextField>,
    pub chat_discovered_popup: Retained<NSPopUpButton>,
    pub chat_gguf_field: Retained<NSTextField>,
    pub chat_prompt_field: Retained<NSTextField>,

    pub sc_toggle_field: Retained<NSTextField>,
    pub sc_ocr_field: Retained<NSTextField>,
    pub sc_ai_field: Retained<NSTextField>,
    pub sc_sel_trans_field: Retained<NSTextField>,
    pub sc_sel_ai_field: Retained<NSTextField>,

    pub opacity_slider: Retained<NSSlider>,
    pub opacity_val_label: Retained<NSTextField>,
    pub autostart_chk: Retained<NSButton>,
}

pub struct AppController {
    pub mtm: MainThreadMarker,
    pub rt: TokioHandle,
    pub ui_tx: UnboundedSender<UiEvent>,
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,

    pub hotkey_mgr: GlobalHotKeyManager,
    pub parsed_shortcuts: ParsedShortcuts,
    pub registered_hotkeys: Vec<HotKey>,

    pub window: Retained<NSWindow>,
    pub effect_view: Retained<NSVisualEffectView>,
    pub tab_translate_btn: Retained<NSButton>,
    pub tab_chat_btn: Retained<NSButton>,
    pub tab_settings_btn: Retained<NSButton>,
    pub status_label: Retained<NSTextField>,
    pub ocr_btn: Retained<NSButton>,
    pub pin_btn: Retained<NSButton>,

    pub translate_ui: TranslateViewWidgets,
    pub chat_ui: ChatViewWidgets,
    pub settings_ui: SettingsViewWidgets,

    pub active_tab: TabMode,
    pub is_pinned: bool,
    pub pin_before_settings: bool,
    pub is_modal_open: bool,

    pub translate_cancel: CancelSlot,
    pub translate_gen: u64,
    pub last_observed_source: String,
    pub last_translated_source: String,
    pub pending_debounce_since: Option<Instant>,

    pub chat_cancel: CancelSlot,
    pub chat_gen: u64,
    pub chat_streaming: bool,
    pub conversations: Vec<Conversation>,
    pub active_conversation_id: Option<usize>,
    pub current_messages: Vec<ChatMessage>,
    pub streaming_assistant_buffer: String,

    pub pending_clear_ai_key: bool,
    pub pending_clear_chat_key: bool,

    pub last_known_size: (f64, f64),
    pub pending_resize_save: Option<Instant>,
}

impl AppController {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        mtm: MainThreadMarker,
        rt: TokioHandle,
        ui_tx: UnboundedSender<UiEvent>,
        settings: Arc<SettingsStore>,
        history: Arc<HistoryStore>,
        hotkey_mgr: GlobalHotKeyManager,
        window: Retained<NSWindow>,
        effect_view: Retained<NSVisualEffectView>,
        dispatcher: &AnyObject,
    ) -> Self {
        let action_sel: Sel = sel!(handleControl:);
        let (win_w, win_h) = settings.window_size();

        // Top Navigation Bar
        let tab_translate_btn = make_button(
            mtm,
            top_rect(win_h, 14.0, 10.0, 74.0, 28.0),
            "● 翻译",
            ActionTag::TabTranslate as isize,
            dispatcher,
            action_sel,
        );
        let tab_chat_btn = make_button(
            mtm,
            top_rect(win_h, 92.0, 10.0, 86.0, 28.0),
            "AI 对话",
            ActionTag::TabChat as isize,
            dispatcher,
            action_sel,
        );
        let tab_settings_btn = make_button(
            mtm,
            top_rect(win_h, 182.0, 10.0, 72.0, 28.0),
            "设置",
            ActionTag::TabSettings as isize,
            dispatcher,
            action_sel,
        );
        let status_label = make_label(
            mtm,
            top_rect(win_h, 264.0, 14.0, (win_w - 445.0).max(120.0), 20.0),
            "纯 Rust 原生极速引擎就绪",
            11.5,
            false,
            true,
        );
        let ocr_btn = make_button(
            mtm,
            top_rect(win_h, win_w - 172.0, 10.0, 86.0, 28.0),
            "截图 OCR",
            ActionTag::TriggerOcr as isize,
            dispatcher,
            action_sel,
        );
        let pin_btn = make_button(
            mtm,
            top_rect(win_h, win_w - 82.0, 10.0, 68.0, 28.0),
            "置顶",
            ActionTag::TogglePin as isize,
            dispatcher,
            action_sel,
        );

        add_subview(&effect_view, &tab_translate_btn);
        add_subview(&effect_view, &tab_chat_btn);
        add_subview(&effect_view, &tab_settings_btn);
        add_subview(&effect_view, &status_label);
        add_subview(&effect_view, &ocr_btn);
        add_subview(&effect_view, &pin_btn);

        let body_h = (win_h - 46.0).max(320.0);
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(win_w, body_h));

        let translate_ui = Self::build_translate_view(mtm, body_frame, dispatcher, action_sel);
        let chat_ui = Self::build_chat_view(mtm, body_frame, dispatcher, action_sel);
        let settings_ui =
            Self::build_settings_view(mtm, body_frame, &settings, dispatcher, action_sel);

        chat_ui.root.setHidden(true);
        settings_ui.root.setHidden(true);

        add_subview(&effect_view, &translate_ui.root);
        add_subview(&effect_view, &chat_ui.root);
        add_subview(&effect_view, &settings_ui.root);

        let sc_cfg = settings.shortcuts();
        let parsed_shortcuts = parse_shortcut_config(&sc_cfg).unwrap_or_default();
        let mut registered_hotkeys = Vec::new();
        for hk in parsed_shortcuts.unique_hotkeys() {
            if hotkey_mgr.register(hk).is_ok() {
                registered_hotkeys.push(hk);
            }
        }

        let conversations = history.read_conversations().unwrap_or_default();
        let (active_conversation_id, current_messages) = match conversations.last() {
            Some(last) => (Some(last.id), last.messages.clone()),
            None => (None, Vec::new()),
        };

        let mut ctrl = Self {
            mtm,
            rt,
            ui_tx,
            settings,
            history,
            hotkey_mgr,
            parsed_shortcuts,
            registered_hotkeys,
            window,
            effect_view,
            tab_translate_btn,
            tab_chat_btn,
            tab_settings_btn,
            status_label,
            ocr_btn,
            pin_btn,
            translate_ui,
            chat_ui,
            settings_ui,
            active_tab: TabMode::Translate,
            is_pinned: false,
            pin_before_settings: false,
            is_modal_open: false,
            translate_cancel: CancelSlot::new(),
            translate_gen: 0,
            last_observed_source: String::new(),
            last_translated_source: String::new(),
            pending_debounce_since: None,
            chat_cancel: CancelSlot::new(),
            chat_gen: 0,
            chat_streaming: false,
            conversations,
            active_conversation_id,
            current_messages,
            streaming_assistant_buffer: String::new(),
            pending_clear_ai_key: false,
            pending_clear_chat_key: false,
            last_known_size: (win_w, win_h),
            pending_resize_save: None,
        };

        ctrl.refresh_ai_header_badge();
        ctrl.refresh_conversations_popup();
        ctrl.render_chat_transcript();
        ctrl
    }

    fn build_translate_view(
        mtm: MainThreadMarker,
        frame: NSRect,
        dispatcher: &AnyObject,
        action_sel: Sel,
    ) -> TranslateViewWidgets {
        let w = frame.size.width;
        let h = frame.size.height;
        let root = NSView::initWithFrame(mtm.alloc(), frame);

        let src_labels: Vec<&str> = SOURCE_LANGUAGES.iter().map(|l| l.label).collect();
        let tgt_labels: Vec<&str> = TARGET_LANGUAGES.iter().map(|l| l.label).collect();

        let source_popup = make_popup(
            mtm,
            top_rect(h, 14.0, 6.0, 145.0, 26.0),
            &src_labels,
            0,
            ActionTag::SourceLangChanged as isize,
            dispatcher,
            action_sel,
        );
        let swap_btn = make_button(
            mtm,
            top_rect(h, 163.0, 6.0, 38.0, 26.0),
            "⇄",
            ActionTag::SwapLanguages as isize,
            dispatcher,
            action_sel,
        );
        let target_popup = make_popup(
            mtm,
            top_rect(h, 205.0, 6.0, 165.0, 26.0),
            &tgt_labels,
            0,
            ActionTag::TargetLangChanged as isize,
            dispatcher,
            action_sel,
        );
        let translate_btn = make_button(
            mtm,
            top_rect(h, w - 166.0, 6.0, 92.0, 26.0),
            "立即翻译",
            ActionTag::TranslateNow as isize,
            dispatcher,
            action_sel,
        );
        let clear_btn = make_button(
            mtm,
            top_rect(h, w - 70.0, 6.0, 56.0, 26.0),
            "清空",
            ActionTag::ClearTranslate as isize,
            dispatcher,
            action_sel,
        );

        let input_h = 118.0;
        let (input_scroll, input_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 14.0, 38.0, w - 28.0, input_h),
            true,
            14.0,
            Some(dispatcher),
        );

        let cards_top = 38.0 + input_h + 10.0;
        let cards_h = (h - cards_top - 14.0).max(160.0);
        let col_w = ((w - 38.0) / 2.0).max(200.0);

        let web_card = make_card_box(mtm, top_rect(h, 14.0, cards_top, col_w, cards_h));
        let web_header_label = make_label(
            mtm,
            top_rect(cards_h, 10.0, 8.0, col_w - 76.0, 18.0),
            "网页极速翻译 (Google / Bing 竞速)",
            12.0,
            true,
            false,
        );
        let web_copy_btn = make_button(
            mtm,
            top_rect(cards_h, col_w - 64.0, 5.0, 54.0, 24.0),
            "复制",
            ActionTag::CopyWebResult as isize,
            dispatcher,
            action_sel,
        );
        let (web_scroll, web_tv) = make_scrollable_text_view(
            mtm,
            top_rect(cards_h, 4.0, 32.0, col_w - 8.0, cards_h - 36.0),
            false,
            13.5,
            None,
        );
        add_subview(&web_card, &web_header_label);
        add_subview(&web_card, &web_copy_btn);
        add_subview(&web_card, &web_scroll);

        let ai_card = make_card_box(
            mtm,
            top_rect(h, 14.0 + col_w + 10.0, cards_top, col_w, cards_h),
        );
        let ai_header_label = make_label(
            mtm,
            top_rect(cards_h, 10.0, 8.0, col_w - 76.0, 18.0),
            "AI 翻译",
            12.0,
            true,
            false,
        );
        let ai_copy_btn = make_button(
            mtm,
            top_rect(cards_h, col_w - 64.0, 5.0, 54.0, 24.0),
            "复制",
            ActionTag::CopyAiResult as isize,
            dispatcher,
            action_sel,
        );
        let (ai_scroll, ai_tv) = make_scrollable_text_view(
            mtm,
            top_rect(cards_h, 4.0, 32.0, col_w - 8.0, cards_h - 36.0),
            false,
            13.5,
            None,
        );
        add_subview(&ai_card, &ai_header_label);
        add_subview(&ai_card, &ai_copy_btn);
        add_subview(&ai_card, &ai_scroll);

        add_subview(&root, &source_popup);
        add_subview(&root, &swap_btn);
        add_subview(&root, &target_popup);
        add_subview(&root, &translate_btn);
        add_subview(&root, &clear_btn);
        add_subview(&root, &input_scroll);
        add_subview(&root, &web_card);
        add_subview(&root, &ai_card);

        TranslateViewWidgets {
            root,
            source_popup,
            swap_btn,
            target_popup,
            translate_btn,
            clear_btn,
            input_scroll,
            input_tv,
            web_card,
            web_header_label,
            web_copy_btn,
            web_scroll,
            web_tv,
            ai_card,
            ai_header_label,
            ai_copy_btn,
            ai_scroll,
            ai_tv,
        }
    }

    fn build_chat_view(
        mtm: MainThreadMarker,
        frame: NSRect,
        dispatcher: &AnyObject,
        action_sel: Sel,
    ) -> ChatViewWidgets {
        let w = frame.size.width;
        let h = frame.size.height;
        let root = NSView::initWithFrame(mtm.alloc(), frame);

        let conv_popup = make_popup(
            mtm,
            top_rect(h, 14.0, 6.0, 260.0, 26.0),
            &["当前对话"],
            0,
            ActionTag::SelectConversation as isize,
            dispatcher,
            action_sel,
        );
        let new_conv_btn = make_button(
            mtm,
            top_rect(h, 280.0, 6.0, 82.0, 26.0),
            "+ 新对话",
            ActionTag::NewConversation as isize,
            dispatcher,
            action_sel,
        );
        let clear_hist_btn = make_button(
            mtm,
            top_rect(h, 366.0, 6.0, 82.0, 26.0),
            "清空历史",
            ActionTag::ClearChatHistory as isize,
            dispatcher,
            action_sel,
        );
        let stop_btn = make_button(
            mtm,
            top_rect(h, w - 166.0, 6.0, 74.0, 26.0),
            "停止",
            ActionTag::StopChatStream as isize,
            dispatcher,
            action_sel,
        );
        let copy_reply_btn = make_button(
            mtm,
            top_rect(h, w - 88.0, 6.0, 74.0, 26.0),
            "复制回复",
            ActionTag::CopyLastReply as isize,
            dispatcher,
            action_sel,
        );

        let input_h = 72.0;
        let transcript_h = (h - 38.0 - input_h - 22.0).max(180.0);
        let (transcript_scroll, transcript_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 14.0, 38.0, w - 28.0, transcript_h),
            false,
            13.5,
            None,
        );
        let input_top = 38.0 + transcript_h + 8.0;
        let (input_scroll, input_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 14.0, input_top, w - 104.0, input_h),
            true,
            13.5,
            Some(dispatcher),
        );
        let send_btn = make_button(
            mtm,
            top_rect(h, w - 84.0, input_top + 18.0, 70.0, 36.0),
            "发送",
            ActionTag::SendChat as isize,
            dispatcher,
            action_sel,
        );

        add_subview(&root, &conv_popup);
        add_subview(&root, &new_conv_btn);
        add_subview(&root, &clear_hist_btn);
        add_subview(&root, &stop_btn);
        add_subview(&root, &copy_reply_btn);
        add_subview(&root, &transcript_scroll);
        add_subview(&root, &input_scroll);
        add_subview(&root, &send_btn);

        ChatViewWidgets {
            root,
            conv_popup,
            new_conv_btn,
            clear_hist_btn,
            stop_btn,
            copy_reply_btn,
            transcript_scroll,
            transcript_tv,
            input_scroll,
            input_tv,
            send_btn,
        }
    }

    fn build_settings_view(
        mtm: MainThreadMarker,
        frame: NSRect,
        settings: &SettingsStore,
        dispatcher: &AnyObject,
        action_sel: Sel,
    ) -> SettingsViewWidgets {
        let w = frame.size.width;
        let h = frame.size.height;
        let root = NSView::initWithFrame(mtm.alloc(), frame);

        let ai_cfg = settings.ai_translation_for_request();
        let chat_cfg = settings.chat_ai_for_request();
        let sc_cfg = settings.shortcuts();
        let app_cfg = settings.appearance();

        let backend_labels: Vec<&str> = AiBackendKind::ALL.iter().map(|b| b.label()).collect();
        let ai_backend_idx = AiBackendKind::ALL
            .iter()
            .position(|&b| b == ai_cfg.backend)
            .unwrap_or(0);
        let chat_backend_idx = AiBackendKind::ALL
            .iter()
            .position(|&b| b == chat_cfg.backend)
            .unwrap_or(0);

        // Left Column: AI 翻译引擎 & AI 对话引擎
        let sec1_title = make_label(
            mtm,
            top_rect(h, 14.0, 4.0, 200.0, 18.0),
            "一、AI 翻译引擎（独立模块）",
            12.5,
            true,
            false,
        );
        let ai_enabled_chk = make_checkbox(
            mtm,
            top_rect(h, 210.0, 3.0, 120.0, 20.0),
            "启用 AI 翻译",
            ai_cfg.enabled,
            0,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &sec1_title);
        add_subview(&root, &ai_enabled_chk);

        let lbl_ai_backend = make_label(mtm, top_rect(h, 14.0, 28.0, 62.0, 18.0), "接入方式", 11.5, false, true);
        let ai_backend_popup = make_popup(
            mtm,
            top_rect(h, 78.0, 25.0, 195.0, 24.0),
            &backend_labels,
            ai_backend_idx,
            ActionTag::AiTransBackendChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_ai_def_url = make_button(
            mtm,
            top_rect(h, 276.0, 25.0, 62.0, 24.0),
            "默认地址",
            ActionTag::AiTransFillDefaultUrl as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_ai_backend);
        add_subview(&root, &ai_backend_popup);
        add_subview(&root, &btn_ai_def_url);

        let lbl_ai_url = make_label(mtm, top_rect(h, 14.0, 55.0, 62.0, 18.0), "服务地址", 11.5, false, true);
        let ai_base_url_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 53.0, 258.0, 22.0),
            &ai_cfg.base_url,
            "http://127.0.0.1:11434 或 https://api.openai.com/v1",
        );
        add_subview(&root, &lbl_ai_url);
        add_subview(&root, &ai_base_url_field);

        let lbl_ai_key = make_label(mtm, top_rect(h, 14.0, 81.0, 62.0, 18.0), "API Key", 11.5, false, true);
        let ai_key_field = make_secure_field(
            mtm,
            top_rect(h, 78.0, 79.0, 148.0, 22.0),
            if ai_cfg.has_api_key { "已保存在配置中 (留空不改)" } else { "本地服务/GGUF 免填" },
        );
        let ai_key_status_label = make_label(
            mtm,
            top_rect(h, 230.0, 81.0, 52.0, 18.0),
            if ai_cfg.has_api_key { "已保存" } else { "未设置" },
            11.0,
            false,
            true,
        );
        let btn_ai_clr_key = make_button(
            mtm,
            top_rect(h, 282.0, 78.0, 56.0, 22.0),
            "清除",
            ActionTag::AiTransClearKey as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_ai_key);
        add_subview(&root, &ai_key_field);
        add_subview(&root, &ai_key_status_label);
        add_subview(&root, &btn_ai_clr_key);

        let lbl_ai_model = make_label(mtm, top_rect(h, 14.0, 107.0, 62.0, 18.0), "模型名称", 11.5, false, true);
        let ai_model_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 105.0, 122.0, 22.0),
            &ai_cfg.model,
            "如 qwen2.5:7b / gpt-4o-mini",
        );
        let btn_ai_discover = make_button(
            mtm,
            top_rect(h, 202.0, 104.0, 64.0, 24.0),
            "探测模型",
            ActionTag::AiTransDiscoverModels as isize,
            dispatcher,
            action_sel,
        );
        let ai_discovered_popup = make_popup(
            mtm,
            top_rect(h, 266.0, 104.0, 72.0, 24.0),
            &["选择…"],
            0,
            ActionTag::AiTransModelPopupSelected as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_ai_model);
        add_subview(&root, &ai_model_field);
        add_subview(&root, &btn_ai_discover);
        add_subview(&root, &ai_discovered_popup);

        let lbl_ai_gguf = make_label(mtm, top_rect(h, 14.0, 133.0, 62.0, 18.0), "GGUF路径", 11.5, false, true);
        let ai_gguf_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 131.0, 198.0, 22.0),
            &ai_cfg.gguf_model_path,
            "直接加载 .gguf 量化模型路径",
        );
        let btn_ai_gguf = make_button(
            mtm,
            top_rect(h, 278.0, 130.0, 60.0, 24.0),
            "浏览…",
            ActionTag::AiTransBrowseGguf as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_ai_gguf);
        add_subview(&root, &ai_gguf_field);
        add_subview(&root, &btn_ai_gguf);

        let lbl_ai_tok = make_label(mtm, top_rect(h, 14.0, 159.0, 62.0, 18.0), "分词器", 11.5, false, true);
        let ai_tokenizer_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 157.0, 198.0, 22.0),
            &ai_cfg.gguf_tokenizer_path,
            "可选 tokenizer.json (留空读内嵌词表)",
        );
        let btn_ai_tok = make_button(
            mtm,
            top_rect(h, 278.0, 156.0, 60.0, 24.0),
            "浏览…",
            ActionTag::AiTransBrowseTokenizer as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_ai_tok);
        add_subview(&root, &ai_tokenizer_field);
        add_subview(&root, &btn_ai_tok);

        let lbl_ai_prompt = make_label(mtm, top_rect(h, 14.0, 185.0, 62.0, 18.0), "翻译指令", 11.5, false, true);
        let ai_prompt_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 183.0, 258.0, 22.0),
            &ai_cfg.translate_prompt,
            "包含 {source} 与 {target} 占位符",
        );
        add_subview(&root, &lbl_ai_prompt);
        add_subview(&root, &ai_prompt_field);

        // Section 2: AI 对话引擎（左下）
        let sec2_title = make_label(
            mtm,
            top_rect(h, 14.0, 216.0, 165.0, 18.0),
            "二、AI 对话引擎",
            12.5,
            true,
            false,
        );
        let btn_sync_from_trans = make_button(
            mtm,
            top_rect(h, 185.0, 212.0, 152.0, 24.0),
            "复用上方翻译模型配置",
            ActionTag::ChatCopyFromTrans as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &sec2_title);
        add_subview(&root, &btn_sync_from_trans);

        let lbl_chat_backend = make_label(mtm, top_rect(h, 14.0, 242.0, 62.0, 18.0), "接入方式", 11.5, false, true);
        let chat_backend_popup = make_popup(
            mtm,
            top_rect(h, 78.0, 239.0, 195.0, 24.0),
            &backend_labels,
            chat_backend_idx,
            ActionTag::ChatBackendChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_chat_def_url = make_button(
            mtm,
            top_rect(h, 276.0, 239.0, 62.0, 24.0),
            "默认地址",
            ActionTag::ChatFillDefaultUrl as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_chat_backend);
        add_subview(&root, &chat_backend_popup);
        add_subview(&root, &btn_chat_def_url);

        let lbl_chat_url = make_label(mtm, top_rect(h, 14.0, 269.0, 62.0, 18.0), "服务地址", 11.5, false, true);
        let chat_base_url_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 267.0, 258.0, 22.0),
            &chat_cfg.base_url,
            "http://127.0.0.1:11434 或 https://api.openai.com/v1",
        );
        add_subview(&root, &lbl_chat_url);
        add_subview(&root, &chat_base_url_field);

        let lbl_chat_key = make_label(mtm, top_rect(h, 14.0, 295.0, 62.0, 18.0), "API Key", 11.5, false, true);
        let chat_key_field = make_secure_field(
            mtm,
            top_rect(h, 78.0, 293.0, 148.0, 22.0),
            if chat_cfg.has_api_key { "已保存在配置中 (留空不改)" } else { "本地服务/GGUF 免填" },
        );
        let chat_key_status_label = make_label(
            mtm,
            top_rect(h, 230.0, 295.0, 52.0, 18.0),
            if chat_cfg.has_api_key { "已保存" } else { "未设置" },
            11.0,
            false,
            true,
        );
        let btn_chat_clr_key = make_button(
            mtm,
            top_rect(h, 282.0, 292.0, 56.0, 22.0),
            "清除",
            ActionTag::ChatClearKey as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_chat_key);
        add_subview(&root, &chat_key_field);
        add_subview(&root, &chat_key_status_label);
        add_subview(&root, &btn_chat_clr_key);

        let lbl_chat_model = make_label(mtm, top_rect(h, 14.0, 321.0, 62.0, 18.0), "模型名称", 11.5, false, true);
        let chat_model_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 319.0, 122.0, 22.0),
            &chat_cfg.model,
            "如 qwen2.5:7b / gpt-4o-mini",
        );
        let btn_chat_discover = make_button(
            mtm,
            top_rect(h, 202.0, 318.0, 64.0, 24.0),
            "探测模型",
            ActionTag::ChatDiscoverModels as isize,
            dispatcher,
            action_sel,
        );
        let chat_discovered_popup = make_popup(
            mtm,
            top_rect(h, 266.0, 318.0, 72.0, 24.0),
            &["选择…"],
            0,
            ActionTag::ChatModelPopupSelected as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_chat_model);
        add_subview(&root, &chat_model_field);
        add_subview(&root, &btn_chat_discover);
        add_subview(&root, &chat_discovered_popup);

        let lbl_chat_gguf = make_label(mtm, top_rect(h, 14.0, 347.0, 62.0, 18.0), "GGUF路径", 11.5, false, true);
        let chat_gguf_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 345.0, 198.0, 22.0),
            &chat_cfg.gguf_model_path,
            "直接加载 .gguf 量化模型路径",
        );
        let btn_chat_gguf = make_button(
            mtm,
            top_rect(h, 278.0, 344.0, 60.0, 24.0),
            "浏览…",
            ActionTag::ChatBrowseGguf as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &lbl_chat_gguf);
        add_subview(&root, &chat_gguf_field);
        add_subview(&root, &btn_chat_gguf);

        let lbl_chat_prompt = make_label(mtm, top_rect(h, 14.0, 373.0, 62.0, 18.0), "系统提示", 11.5, false, true);
        let chat_prompt_field = make_text_field(
            mtm,
            top_rect(h, 78.0, 371.0, 258.0, 22.0),
            &chat_cfg.chat_prompt,
            "可选 System Prompt",
        );
        add_subview(&root, &lbl_chat_prompt);
        add_subview(&root, &chat_prompt_field);

        // Right Column: 全局快捷键 & 外观通用
        let rx = 354.0;
        let rw = (w - rx - 14.0).max(290.0);

        let sec3_title = make_label(
            mtm,
            top_rect(h, rx, 4.0, 180.0, 18.0),
            "三、全局快捷键",
            12.5,
            true,
            false,
        );
        let btn_reset_sc = make_button(
            mtm,
            top_rect(h, rx + rw - 92.0, 2.0, 92.0, 24.0),
            "恢复默认热键",
            ActionTag::ResetDefaultShortcuts as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &sec3_title);
        add_subview(&root, &btn_reset_sc);

        let lbl_sc1 = make_label(mtm, top_rect(h, rx, 32.0, 88.0, 18.0), "呼出翻译窗口", 11.5, false, true);
        let sc_toggle_field = make_text_field(
            mtm,
            top_rect(h, rx + 90.0, 30.0, rw - 90.0, 22.0),
            &sc_cfg.toggle,
            "如 CmdOrCtrl+Shift+Space",
        );
        let lbl_sc2 = make_label(mtm, top_rect(h, rx, 60.0, 88.0, 18.0), "截图 OCR 翻译", 11.5, false, true);
        let sc_ocr_field = make_text_field(
            mtm,
            top_rect(h, rx + 90.0, 58.0, rw - 90.0, 22.0),
            &sc_cfg.ocr,
            "如 CmdOrCtrl+Shift+S",
        );
        let lbl_sc3 = make_label(mtm, top_rect(h, rx, 88.0, 88.0, 18.0), "呼出 AI 对话", 11.5, false, true);
        let sc_ai_field = make_text_field(
            mtm,
            top_rect(h, rx + 90.0, 86.0, rw - 90.0, 22.0),
            &sc_cfg.ai_dialog,
            "如 Option+Space",
        );
        let lbl_sc4 = make_label(mtm, top_rect(h, rx, 116.0, 88.0, 18.0), "划词即时翻译", 11.5, false, true);
        let sc_sel_trans_field = make_text_field(
            mtm,
            top_rect(h, rx + 90.0, 114.0, rw - 90.0, 22.0),
            &sc_cfg.selection_translate,
            "可与呼出翻译共用同一热键",
        );
        let lbl_sc5 = make_label(mtm, top_rect(h, rx, 144.0, 88.0, 18.0), "划词发送 AI", 11.5, false, true);
        let sc_sel_ai_field = make_text_field(
            mtm,
            top_rect(h, rx + 90.0, 142.0, rw - 90.0, 22.0),
            &sc_cfg.selection_ai_dialog,
            "可与呼出 AI 对话共用同一热键",
        );

        add_subview(&root, &lbl_sc1);
        add_subview(&root, &sc_toggle_field);
        add_subview(&root, &lbl_sc2);
        add_subview(&root, &sc_ocr_field);
        add_subview(&root, &lbl_sc3);
        add_subview(&root, &sc_ai_field);
        add_subview(&root, &lbl_sc4);
        add_subview(&root, &sc_sel_trans_field);
        add_subview(&root, &lbl_sc5);
        add_subview(&root, &sc_sel_ai_field);

        let sec4_title = make_label(
            mtm,
            top_rect(h, rx, 182.0, 200.0, 18.0),
            "四、外观与系统集成",
            12.5,
            true,
            false,
        );
        add_subview(&root, &sec4_title);

        let lbl_opacity = make_label(mtm, top_rect(h, rx, 210.0, 82.0, 18.0), "面板不透明度", 11.5, false, true);
        let opacity_slider = make_slider(
            mtm,
            top_rect(h, rx + 86.0, 208.0, rw - 136.0, 22.0),
            PANEL_OPACITY_MIN,
            PANEL_OPACITY_MAX,
            app_cfg.panel_opacity,
            ActionTag::OpacitySliderChanged as isize,
            dispatcher,
            action_sel,
        );
        let opacity_val_label = make_label(
            mtm,
            top_rect(h, rx + rw - 46.0, 210.0, 46.0, 18.0),
            &format!("{:.0}%", app_cfg.panel_opacity * 100.0),
            11.5,
            false,
            false,
        );
        add_subview(&root, &lbl_opacity);
        add_subview(&root, &opacity_slider);
        add_subview(&root, &opacity_val_label);

        let autostart_chk = make_checkbox(
            mtm,
            top_rect(h, rx, 240.0, 220.0, 22.0),
            "登录 macOS 时开机自动静默启动",
            ttree_platform_macos::autostart::is_enabled(),
            0,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &autostart_chk);

        let notes_label = make_label(
            mtm,
            top_rect(h, rx, 272.0, rw, 84.0),
            "提示：\n• Ollama 默认端口 11434，LM Studio 默认 1234，MLX 默认 8080\n• 选择「直接加载本地 GGUF 模型」可在 Apple Silicon Metal GPU 上原生运行量化模型\n• 修改后点击下方「保存全部配置」立即生效",
            11.0,
            false,
            true,
        );
        add_subview(&root, &notes_label);

        let save_btn = make_button(
            mtm,
            top_rect(h, rx, 364.0, rw, 34.0),
            "💾 保存全部配置 (立即生效)",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&root, &save_btn);

        SettingsViewWidgets {
            root,
            ai_enabled_chk,
            ai_backend_popup,
            ai_base_url_field,
            ai_key_field,
            ai_key_status_label,
            ai_model_field,
            ai_discovered_popup,
            ai_gguf_field,
            ai_tokenizer_field,
            ai_prompt_field,
            chat_backend_popup,
            chat_base_url_field,
            chat_key_field,
            chat_key_status_label,
            chat_model_field,
            chat_discovered_popup,
            chat_gguf_field,
            chat_prompt_field,
            sc_toggle_field,
            sc_ocr_field,
            sc_ai_field,
            sc_sel_trans_field,
            sc_sel_ai_field,
            opacity_slider,
            opacity_val_label,
            autostart_chk,
        }
    }

    pub fn set_status(&self, msg: &str) {
        set_field_string(&self.status_label, msg);
    }

    pub fn switch_tab(&mut self, tab: TabMode) {
        if self.active_tab == TabMode::Settings && tab != TabMode::Settings {
            self.set_pinned(self.pin_before_settings);
        } else if self.active_tab != TabMode::Settings && tab == TabMode::Settings {
            self.pin_before_settings = self.is_pinned;
            self.set_pinned(true);
        }

        self.active_tab = tab;
        self.translate_ui.root.setHidden(tab != TabMode::Translate);
        self.chat_ui.root.setHidden(tab != TabMode::Chat);
        self.settings_ui.root.setHidden(tab != TabMode::Settings);

        let t_title = if tab == TabMode::Translate { "● 翻译" } else { "翻译" };
        let c_title = if tab == TabMode::Chat { "● AI 对话" } else { "AI 对话" };
        let s_title = if tab == TabMode::Settings { "● 设置" } else { "设置" };
        self.tab_translate_btn.setTitle(&NSString::from_str(t_title));
        self.tab_chat_btn.setTitle(&NSString::from_str(c_title));
        self.tab_settings_btn.setTitle(&NSString::from_str(s_title));

        match tab {
            TabMode::Translate => {
                let _ = self.window.makeFirstResponder(Some(&self.translate_ui.input_tv));
            }
            TabMode::Chat => {
                let _ = self.window.makeFirstResponder(Some(&self.chat_ui.input_tv));
            }
            TabMode::Settings => {}
        }
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.is_pinned = pinned;
        let level = if pinned {
            NSFloatingWindowLevel
        } else {
            NSNormalWindowLevel
        };
        self.window.setLevel(level);
        let title = if pinned { "📌 已置顶" } else { "置顶" };
        self.pin_btn.setTitle(&NSString::from_str(title));
    }

    pub fn show_and_focus_window(&mut self) {
        let app = NSApplication::sharedApplication(self.mtm);
        if !self.window.isVisible() {
            if let Some(screen) = NSScreen::mainScreen(self.mtm) {
                let vf = screen.visibleFrame();
                let wf = self.window.frame();
                let x = vf.origin.x + (vf.size.width - wf.size.width) * 0.5;
                let y = vf.origin.y + (vf.size.height - wf.size.height) * 0.62;
                self.window.setFrameOrigin(NSPoint::new(x, y));
            }
        }
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        self.window.makeKeyAndOrderFront(None);
        match self.active_tab {
            TabMode::Translate => {
                let _ = self.window.makeFirstResponder(Some(&self.translate_ui.input_tv));
            }
            TabMode::Chat => {
                let _ = self.window.makeFirstResponder(Some(&self.chat_ui.input_tv));
            }
            TabMode::Settings => {}
        }
    }

    pub fn hide_window(&mut self) {
        self.window.orderOut(None);
    }

    pub fn refresh_ai_header_badge(&self) {
        let cfg = self.settings.ai_translation_for_request();
        let label = if !cfg.enabled {
            "AI 翻译 (已在设置中关闭)".to_string()
        } else {
            match cfg.backend {
                AiBackendKind::OpenAiCompatible => format!("AI 翻译 (云端 · {})", cfg.model),
                AiBackendKind::Ollama => format!("AI 翻译 (Ollama · {})", cfg.model),
                AiBackendKind::LocalServer => format!("AI 翻译 (本地服务 · {})", cfg.model),
                AiBackendKind::CandleGguf => {
                    let name = std::path::Path::new(&cfg.gguf_model_path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("未选 .gguf");
                    format!("AI 翻译 (GGUF · {name})")
                }
            }
        };
        set_field_string(&self.translate_ui.ai_header_label, &label);
    }

    pub fn selected_source_code(&self) -> &'static str {
        let idx = self.translate_ui.source_popup.indexOfSelectedItem();
        SOURCE_LANGUAGES
            .get(idx.max(0) as usize)
            .map(|l| l.code)
            .unwrap_or("auto")
    }

    pub fn selected_target_code(&self) -> &'static str {
        let idx = self.translate_ui.target_popup.indexOfSelectedItem();
        TARGET_LANGUAGES
            .get(idx.max(0) as usize)
            .map(|l| l.code)
            .unwrap_or(ADAPTIVE_TARGET)
    }

    pub fn start_translation(&mut self) {
        let raw_text = get_text_view_string(&self.translate_ui.input_tv);
        let trimmed = raw_text.trim().to_string();
        self.pending_debounce_since = None;

        if trimmed.is_empty() {
            self.translate_cancel.cancel_current();
            set_text_view_string(&self.translate_ui.web_tv, "");
            set_text_view_string(&self.translate_ui.ai_tv, "");
            self.last_translated_source.clear();
            self.set_status("就绪");
            return;
        }

        let cancel_token = self.translate_cancel.next();
        self.translate_gen = self.translate_gen.wrapping_add(1);
        let gen = self.translate_gen;
        self.last_translated_source = trimmed.clone();

        let src_ui = self.selected_source_code();
        let tgt_ui = self.selected_target_code();
        let (resolved_src, resolved_tgt) = resolve_language_pair(&trimmed, src_ui, tgt_ui);

        set_text_view_string(&self.translate_ui.web_tv, "正在极速翻译…");
        let ai_cfg = self.settings.ai_translation_for_request();
        self.refresh_ai_header_badge();
        if ai_cfg.enabled {
            set_text_view_string(&self.translate_ui.ai_tv, "正在请求 AI 流式译文…");
        } else {
            set_text_view_string(&self.translate_ui.ai_tv, "（AI 翻译未启用，可在「设置」中开启或接入本地 Ollama / GGUF 模型）");
        }
        self.set_status(&format!("正在翻译 ({resolved_src} → {resolved_tgt})…"));

        // 1. 并发启动极速网页翻译（Google POST + Bing 内存缓存 Token 竞速）
        let web_tx = self.ui_tx.clone();
        let web_text = trimmed.clone();
        let web_src = resolved_src.clone();
        let web_tgt = resolved_tgt.clone();
        let web_cancel = cancel_token.clone();
        self.rt.spawn(async move {
            let res =
                ttree_translate::translate_web(&web_text, &web_src, &web_tgt, &web_cancel).await;
            if !web_cancel.is_cancelled() {
                let _ = web_tx.send(UiEvent::WebTranslateResult { gen, result: res });
            }
        });

        // 2. 并发启动 AI 流式翻译（云端 / Ollama / LM Studio / 本地 GGUF）
        if ai_cfg.enabled {
            let ai_tx = self.ui_tx.clone();
            let ai_text = trimmed;
            let ai_cancel = cancel_token;
            self.rt.spawn(async move {
                let delta_tx = ai_tx.clone();
                let res = ttree_ai::stream_translate(
                    &ai_cfg,
                    &ai_text,
                    &resolved_src,
                    &resolved_tgt,
                    &ai_cancel,
                    move |delta| {
                        let _ = delta_tx.send(UiEvent::AiTranslateDelta {
                            gen,
                            delta: delta.to_string(),
                        });
                    },
                )
                .await;
                if !ai_cancel.is_cancelled() {
                    let _ = ai_tx.send(UiEvent::AiTranslateFinished {
                        gen,
                        error: res.err(),
                    });
                }
            });
        }
    }

    pub fn trigger_ocr(&mut self) {
        self.hide_window();
        self.set_status("正在框选截图区域…");
        let tx = self.ui_tx.clone();
        self.rt.spawn(async move {
            let res = tokio::task::spawn_blocking(ttree_ocr::capture_and_recognize)
                .await
                .unwrap_or_else(|e| Err(format!("OCR 线程异常: {e}")));
            let _ = tx.send(UiEvent::OcrCompleted { outcome: res });
        });
    }

    pub fn handle_hotkey_action(&mut self, action: ShortcutAction) {
        let sc = self.settings.shortcuts();
        match action {
            ShortcutAction::Toggle => {
                if self.window.isVisible()
                    && self.window.isKeyWindow()
                    && self.active_tab == TabMode::Translate
                {
                    self.hide_window();
                } else {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                }
            }
            ShortcutAction::Ocr => {
                self.trigger_ocr();
            }
            ShortcutAction::AiDialog => {
                if self.window.isVisible()
                    && self.window.isKeyWindow()
                    && self.active_tab == TabMode::Chat
                {
                    self.hide_window();
                } else {
                    self.switch_tab(TabMode::Chat);
                    self.show_and_focus_window();
                }
            }
            ShortcutAction::SelectionTranslate => {
                let shared_with_toggle = sc.toggle.trim() == sc.selection_translate.trim();
                if shared_with_toggle
                    && self.window.isVisible()
                    && self.window.isKeyWindow()
                    && self.active_tab == TabMode::Translate
                {
                    self.hide_window();
                    return;
                }
                let tx = self.ui_tx.clone();
                self.rt.spawn(async move {
                    let captured = tokio::task::spawn_blocking(
                        ttree_platform_macos::selection::capture_selected_text,
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten();
                    let _ = tx.send(UiEvent::SelectionCapturedForTranslate {
                        text: captured,
                        fallback_toggle: shared_with_toggle,
                    });
                });
            }
            ShortcutAction::SelectionAiDialog => {
                let shared_with_ai = sc.ai_dialog.trim() == sc.selection_ai_dialog.trim();
                if shared_with_ai
                    && self.window.isVisible()
                    && self.window.isKeyWindow()
                    && self.active_tab == TabMode::Chat
                {
                    self.hide_window();
                    return;
                }
                let tx = self.ui_tx.clone();
                self.rt.spawn(async move {
                    let captured = tokio::task::spawn_blocking(
                        ttree_platform_macos::selection::capture_selected_text,
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten();
                    let _ = tx.send(UiEvent::SelectionCapturedForChat {
                        text: captured,
                        fallback_toggle: shared_with_ai,
                    });
                });
            }
        }
    }

    pub fn on_action(&mut self, tag: ActionTag) {
        match tag {
            ActionTag::TabTranslate => {
                self.switch_tab(TabMode::Translate);
                self.show_and_focus_window();
            }
            ActionTag::TabChat => {
                self.switch_tab(TabMode::Chat);
                self.show_and_focus_window();
            }
            ActionTag::TabSettings => {
                self.switch_tab(TabMode::Settings);
                self.show_and_focus_window();
            }
            ActionTag::TriggerOcr => self.trigger_ocr(),
            ActionTag::TogglePin => self.set_pinned(!self.is_pinned),
            ActionTag::TrayQuit => {
                let app = NSApplication::sharedApplication(self.mtm);
                app.terminate(None);
            }
            ActionTag::HideOrEscape => {
                if self.active_tab == TabMode::Settings {
                    self.switch_tab(TabMode::Translate);
                } else {
                    self.hide_window();
                }
            }
            ActionTag::TranslateNow => self.start_translation(),
            ActionTag::ClearTranslate => {
                self.translate_cancel.cancel_current();
                set_text_view_string(&self.translate_ui.input_tv, "");
                set_text_view_string(&self.translate_ui.web_tv, "");
                set_text_view_string(&self.translate_ui.ai_tv, "");
                self.last_observed_source.clear();
                self.last_translated_source.clear();
                self.pending_debounce_since = None;
                self.set_status("已清空");
            }
            ActionTag::SwapLanguages => {
                let src_code = self.selected_source_code();
                let tgt_code = self.selected_target_code();
                if src_code != "auto" && tgt_code != ADAPTIVE_TARGET {
                    let new_src_code = if tgt_code.starts_with("zh") {
                        "zh-CN"
                    } else {
                        tgt_code
                    };
                    if let Some(new_src_idx) =
                        SOURCE_LANGUAGES.iter().position(|l| l.code == new_src_code)
                    {
                        if let Some(new_tgt_idx) =
                            TARGET_LANGUAGES.iter().position(|l| l.code == src_code)
                        {
                            self.translate_ui
                                .source_popup
                                .selectItemAtIndex(new_src_idx as isize);
                            self.translate_ui
                                .target_popup
                                .selectItemAtIndex(new_tgt_idx as isize);
                            self.start_translation();
                        }
                    }
                }
            }
            ActionTag::SourceLangChanged | ActionTag::TargetLangChanged => {
                if !get_text_view_string(&self.translate_ui.input_tv)
                    .trim()
                    .is_empty()
                {
                    self.start_translation();
                }
            }
            ActionTag::CopyWebResult => {
                let text = get_text_view_string(&self.translate_ui.web_tv);
                if copy_to_pasteboard(&text) {
                    self.set_status("✓ 已复制网页翻译结果");
                }
            }
            ActionTag::CopyAiResult => {
                let text = get_text_view_string(&self.translate_ui.ai_tv);
                if copy_to_pasteboard(&text) {
                    self.set_status("✓ 已复制 AI 翻译结果");
                }
            }
            ActionTag::SendChat => self.send_chat_from_input(),
            ActionTag::NewConversation => {
                self.chat_cancel.cancel_current();
                self.chat_streaming = false;
                let _ = self.history.start_new_conversation();
                self.active_conversation_id = None;
                self.current_messages.clear();
                self.streaming_assistant_buffer.clear();
                self.refresh_conversations_popup();
                self.render_chat_transcript();
                self.set_status("已开启新对话");
            }
            ActionTag::ClearChatHistory => {
                self.chat_cancel.cancel_current();
                self.chat_streaming = false;
                let _ = self.history.clear();
                self.conversations.clear();
                self.active_conversation_id = None;
                self.current_messages.clear();
                self.streaming_assistant_buffer.clear();
                self.refresh_conversations_popup();
                self.render_chat_transcript();
                self.set_status("✓ 对话历史已清空");
            }
            ActionTag::StopChatStream => {
                self.chat_cancel.cancel_current();
                self.chat_streaming = false;
                self.set_status("已停止生成");
            }
            ActionTag::SelectConversation => {
                let idx = self.chat_ui.conv_popup.indexOfSelectedItem();
                if idx >= 0 && (idx as usize) < self.conversations.len() {
                    let conv = &self.conversations[idx as usize];
                    self.active_conversation_id = Some(conv.id);
                    self.current_messages = conv.messages.clone();
                    self.streaming_assistant_buffer.clear();
                    self.render_chat_transcript();
                }
            }
            ActionTag::CopyLastReply => {
                if let Some(last) = self
                    .current_messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant")
                {
                    if copy_to_pasteboard(&last.content) {
                        self.set_status("✓ 已复制最后一条 AI 回复");
                    }
                }
            }
            ActionTag::AiTransBackendChanged => {
                let backend = self.selected_ai_trans_backend();
                let cur_url = get_field_string(&self.settings_ui.ai_base_url_field);
                if cur_url.trim().is_empty()
                    || cur_url == "https://api.openai.com/v1"
                    || cur_url == "http://127.0.0.1:11434"
                    || cur_url == "http://127.0.0.1:1234/v1"
                {
                    set_field_string(
                        &self.settings_ui.ai_base_url_field,
                        backend.default_base_url(),
                    );
                }
            }
            ActionTag::AiTransFillDefaultUrl => {
                let backend = self.selected_ai_trans_backend();
                set_field_string(
                    &self.settings_ui.ai_base_url_field,
                    backend.default_base_url(),
                );
            }
            ActionTag::AiTransDiscoverModels => self.discover_models(false),
            ActionTag::AiTransModelPopupSelected => {
                if let Some(title) = self.settings_ui.ai_discovered_popup.titleOfSelectedItem() {
                    let s = title.to_string();
                    if s != "选择…" {
                        set_field_string(&self.settings_ui.ai_model_field, &s);
                    }
                }
            }
            ActionTag::AiTransBrowseGguf => {
                if let Some(path) = self.pick_file(&["gguf"]) {
                    set_field_string(&self.settings_ui.ai_gguf_field, &path);
                    self.settings_ui.ai_backend_popup.selectItemAtIndex(3);
                }
            }
            ActionTag::AiTransBrowseTokenizer => {
                if let Some(path) = self.pick_file(&["json"]) {
                    set_field_string(&self.settings_ui.ai_tokenizer_field, &path);
                }
            }
            ActionTag::AiTransClearKey => {
                self.pending_clear_ai_key = true;
                set_field_string(&self.settings_ui.ai_key_field, "");
                set_field_string(&self.settings_ui.ai_key_status_label, "待清除");
            }
            ActionTag::ChatBackendChanged => {
                let backend = self.selected_chat_backend();
                let cur_url = get_field_string(&self.settings_ui.chat_base_url_field);
                if cur_url.trim().is_empty()
                    || cur_url == "https://api.openai.com/v1"
                    || cur_url == "http://127.0.0.1:11434"
                    || cur_url == "http://127.0.0.1:1234/v1"
                {
                    set_field_string(
                        &self.settings_ui.chat_base_url_field,
                        backend.default_base_url(),
                    );
                }
            }
            ActionTag::ChatFillDefaultUrl => {
                let backend = self.selected_chat_backend();
                set_field_string(
                    &self.settings_ui.chat_base_url_field,
                    backend.default_base_url(),
                );
            }
            ActionTag::ChatDiscoverModels => self.discover_models(true),
            ActionTag::ChatModelPopupSelected => {
                if let Some(title) = self.settings_ui.chat_discovered_popup.titleOfSelectedItem() {
                    let s = title.to_string();
                    if s != "选择…" {
                        set_field_string(&self.settings_ui.chat_model_field, &s);
                    }
                }
            }
            ActionTag::ChatBrowseGguf => {
                if let Some(path) = self.pick_file(&["gguf"]) {
                    set_field_string(&self.settings_ui.chat_gguf_field, &path);
                    self.settings_ui.chat_backend_popup.selectItemAtIndex(3);
                }
            }
            ActionTag::ChatCopyFromTrans => {
                let idx = self.settings_ui.ai_backend_popup.indexOfSelectedItem();
                self.settings_ui.chat_backend_popup.selectItemAtIndex(idx);
                set_field_string(
                    &self.settings_ui.chat_base_url_field,
                    &get_field_string(&self.settings_ui.ai_base_url_field),
                );
                set_field_string(
                    &self.settings_ui.chat_model_field,
                    &get_field_string(&self.settings_ui.ai_model_field),
                );
                set_field_string(
                    &self.settings_ui.chat_gguf_field,
                    &get_field_string(&self.settings_ui.ai_gguf_field),
                );
                self.set_status("已将翻译模型配置同步至对话引擎表单");
            }
            ActionTag::ChatClearKey => {
                self.pending_clear_chat_key = true;
                set_field_string(&self.settings_ui.chat_key_field, "");
                set_field_string(&self.settings_ui.chat_key_status_label, "待清除");
            }
            ActionTag::ResetDefaultShortcuts => {
                let def = ShortcutConfig::default();
                set_field_string(&self.settings_ui.sc_toggle_field, &def.toggle);
                set_field_string(&self.settings_ui.sc_ocr_field, &def.ocr);
                set_field_string(&self.settings_ui.sc_ai_field, &def.ai_dialog);
                set_field_string(
                    &self.settings_ui.sc_sel_trans_field,
                    &def.selection_translate,
                );
                set_field_string(&self.settings_ui.sc_sel_ai_field, &def.selection_ai_dialog);
                self.set_status("已填入默认快捷键，点击「保存全部配置」生效");
            }
            ActionTag::OpacitySliderChanged => {
                let val = AppearanceConfig::clamp_opacity(
                    self.settings_ui.opacity_slider.doubleValue(),
                );
                self.window.setAlphaValue(val);
                set_field_string(
                    &self.settings_ui.opacity_val_label,
                    &format!("{:.0}%", val * 100.0),
                );
            }
            ActionTag::SaveSettings => self.save_all_settings(),
        }
    }

    fn selected_ai_trans_backend(&self) -> AiBackendKind {
        let idx = self.settings_ui.ai_backend_popup.indexOfSelectedItem();
        AiBackendKind::ALL
            .get(idx.max(0) as usize)
            .copied()
            .unwrap_or(AiBackendKind::OpenAiCompatible)
    }

    fn selected_chat_backend(&self) -> AiBackendKind {
        let idx = self.settings_ui.chat_backend_popup.indexOfSelectedItem();
        AiBackendKind::ALL
            .get(idx.max(0) as usize)
            .copied()
            .unwrap_or(AiBackendKind::OpenAiCompatible)
    }

    fn discover_models(&mut self, for_chat: bool) {
        let (backend, base_url) = if for_chat {
            (
                self.selected_chat_backend(),
                get_field_string(&self.settings_ui.chat_base_url_field),
            )
        } else {
            (
                self.selected_ai_trans_backend(),
                get_field_string(&self.settings_ui.ai_base_url_field),
            )
        };
        self.set_status("正在探测本机已加载模型…");
        let tx = self.ui_tx.clone();
        self.rt.spawn(async move {
            let res = ttree_ai::discover_local_models(backend, &base_url).await;
            let _ = tx.send(UiEvent::ModelsDiscovered {
                for_chat,
                result: res,
            });
        });
    }

    fn pick_file(&mut self, allowed_exts: &[&str]) -> Option<String> {
        self.is_modal_open = true;
        let panel = NSOpenPanel::openPanel(self.mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        let response = panel.runModal();
        self.is_modal_open = false;
        if response != NSModalResponseOK {
            return None;
        }
        let urls = panel.URLs();
        let url = urls.firstObject()?;
        let path = url.path()?.to_string();
        if !allowed_exts.is_empty() {
            let ext_ok = std::path::Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| allowed_exts.iter().any(|a| a.eq_ignore_ascii_case(ext)));
            if !ext_ok {
                self.set_status(&format!(
                    "⚠️ 请选择 .{} 格式的文件",
                    allowed_exts.join("/")
                ));
                return None;
            }
        }
        Some(path)
    }

    fn save_all_settings(&mut self) {
        let ai_input = AiTranslationConfig {
            enabled: self.settings_ui.ai_enabled_chk.state() == NSControlStateValueOn,
            backend: self.selected_ai_trans_backend(),
            base_url: get_field_string(&self.settings_ui.ai_base_url_field),
            has_api_key: false,
            api_key: get_field_string(&self.settings_ui.ai_key_field),
            clear_api_key: self.pending_clear_ai_key,
            model: get_field_string(&self.settings_ui.ai_model_field),
            gguf_model_path: get_field_string(&self.settings_ui.ai_gguf_field),
            gguf_tokenizer_path: get_field_string(&self.settings_ui.ai_tokenizer_field),
            translate_prompt: get_field_string(&self.settings_ui.ai_prompt_field),
        };
        if let Err(e) = self.settings.save_ai_translation(&ai_input) {
            self.set_status(&format!("❌ AI 翻译配置错误: {e}"));
            return;
        }
        self.pending_clear_ai_key = false;

        let chat_input = ChatAiConfig {
            backend: self.selected_chat_backend(),
            base_url: get_field_string(&self.settings_ui.chat_base_url_field),
            has_api_key: false,
            api_key: get_field_string(&self.settings_ui.chat_key_field),
            clear_api_key: self.pending_clear_chat_key,
            model: get_field_string(&self.settings_ui.chat_model_field),
            gguf_model_path: get_field_string(&self.settings_ui.chat_gguf_field),
            gguf_tokenizer_path: get_field_string(&self.settings_ui.ai_tokenizer_field),
            chat_prompt: get_field_string(&self.settings_ui.chat_prompt_field),
        };
        if let Err(e) = self.settings.save_chat_ai(&chat_input) {
            self.set_status(&format!("❌ AI 对话配置错误: {e}"));
            return;
        }
        self.pending_clear_chat_key = false;

        let sc_input = ShortcutConfig {
            toggle: get_field_string(&self.settings_ui.sc_toggle_field),
            ocr: get_field_string(&self.settings_ui.sc_ocr_field),
            ai_dialog: get_field_string(&self.settings_ui.sc_ai_field),
            selection_translate: get_field_string(&self.settings_ui.sc_sel_trans_field),
            selection_ai_dialog: get_field_string(&self.settings_ui.sc_sel_ai_field),
        };
        match parse_shortcut_config(&sc_input) {
            Ok(parsed) => {
                for old_hk in self.registered_hotkeys.drain(..) {
                    let _ = self.hotkey_mgr.unregister(old_hk);
                }
                for hk in parsed.unique_hotkeys() {
                    if self.hotkey_mgr.register(hk).is_ok() {
                        self.registered_hotkeys.push(hk);
                    }
                }
                self.parsed_shortcuts = parsed;
                let _ = self.settings.save_shortcuts(&sc_input);
            }
            Err(e) => {
                self.set_status(&format!("❌ 快捷键无效: {e}"));
                return;
            }
        }

        let opacity =
            AppearanceConfig::clamp_opacity(self.settings_ui.opacity_slider.doubleValue());
        let _ = self.settings.save_appearance(AppearanceConfig {
            panel_opacity: opacity,
        });
        self.window.setAlphaValue(opacity);

        let want_autostart = self.settings_ui.autostart_chk.state() == NSControlStateValueOn;
        if want_autostart != ttree_platform_macos::autostart::is_enabled() {
            let _ = ttree_platform_macos::autostart::set_enabled(want_autostart);
        }

        let saved_ai = self.settings.ai_translation_for_request();
        let saved_chat = self.settings.chat_ai_for_request();
        set_field_string(&self.settings_ui.ai_key_field, "");
        set_field_string(
            &self.settings_ui.ai_key_status_label,
            if saved_ai.has_api_key { "已保存" } else { "未设置" },
        );
        set_field_string(&self.settings_ui.chat_key_field, "");
        set_field_string(
            &self.settings_ui.chat_key_status_label,
            if saved_chat.has_api_key { "已保存" } else { "未设置" },
        );
        self.refresh_ai_header_badge();
        self.set_status("✓ 全部配置已保存并立即生效");
    }

    pub fn send_chat_from_input(&mut self) {
        let raw = get_text_view_string(&self.chat_ui.input_tv);
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        set_text_view_string(&self.chat_ui.input_tv, "");
        self.send_chat_text(trimmed);
    }

    pub fn send_chat_text(&mut self, text: String) {
        let cancel_token = self.chat_cancel.next();
        self.chat_gen = self.chat_gen.wrapping_add(1);
        let gen = self.chat_gen;

        self.current_messages.push(ChatMessage::user(text));
        self.streaming_assistant_buffer.clear();
        self.chat_streaming = true;
        self.render_chat_transcript();
        self.set_status("AI 正在思考并流式回复…");

        let cfg = self.settings.chat_ai_for_request();
        let context = recent_chat_context(&self.current_messages);
        let tx = self.ui_tx.clone();

        self.rt.spawn(async move {
            let delta_tx = tx.clone();
            let res = ttree_ai::stream_chat(&cfg, &context, &cancel_token, move |delta| {
                let _ = delta_tx.send(UiEvent::ChatDelta {
                    gen,
                    delta: delta.to_string(),
                });
            })
            .await;
            if !cancel_token.is_cancelled() {
                let _ = tx.send(UiEvent::ChatFinished {
                    gen,
                    error: res.err(),
                });
            }
        });
    }

    pub fn refresh_conversations_popup(&mut self) {
        self.conversations = self.history.read_conversations().unwrap_or_default();
        self.chat_ui.conv_popup.removeAllItems();
        if self.conversations.is_empty() {
            self.chat_ui
                .conv_popup
                .addItemWithTitle(&NSString::from_str("当前新对话"));
            return;
        }
        for (i, conv) in self.conversations.iter().enumerate() {
            let title = if conv.summary.is_empty() {
                format!("对话 #{}", i + 1)
            } else {
                format!("{}. {}", i + 1, conv.summary)
            };
            self.chat_ui
                .conv_popup
                .addItemWithTitle(&NSString::from_str(&title));
        }
        if let Some(active_id) = self.active_conversation_id {
            if let Some(pos) = self.conversations.iter().position(|c| c.id == active_id) {
                self.chat_ui.conv_popup.selectItemAtIndex(pos as isize);
            }
        } else {
            self.chat_ui
                .conv_popup
                .addItemWithTitle(&NSString::from_str("● 当前新对话"));
            let count = self.chat_ui.conv_popup.numberOfItems();
            self.chat_ui.conv_popup.selectItemAtIndex(count - 1);
        }
    }

    pub fn render_chat_transcript(&self) {
        let mut out = String::new();
        if self.current_messages.is_empty() && self.streaming_assistant_buffer.is_empty() {
            out.push_str("欢迎使用 TTREE AI 对话（支持云端 OpenAI / 本机 Ollama / LM Studio / 直接加载 .gguf 模型）。\n在下方输入框按 Enter 发送，Shift+Enter 换行。");
        } else {
            for msg in &self.current_messages {
                let prefix = if msg.role == "user" {
                    "【你】"
                } else {
                    "【AI】"
                };
                out.push_str(prefix);
                out.push('\n');
                out.push_str(&msg.content);
                out.push_str("\n\n");
            }
            if self.chat_streaming {
                out.push_str("【AI】\n");
                out.push_str(&self.streaming_assistant_buffer);
                out.push_str(" ▍");
            }
        }
        let s = NSString::from_str(&out);
        self.chat_ui.transcript_tv.setString(&s);
        let len = s.length();
        self.chat_ui
            .transcript_tv
            .scrollRangeToVisible(NSRange::new(len, 0));
    }

    pub fn handle_ui_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::WebTranslateResult { gen, result } => {
                if gen != self.translate_gen {
                    return;
                }
                match result {
                    Ok(translated) => {
                        set_text_view_string(&self.translate_ui.web_tv, &translated);
                        self.set_status("✓ 网页翻译完成");
                    }
                    Err(err) => {
                        set_text_view_string(
                            &self.translate_ui.web_tv,
                            &format!("⚠️ 网页翻译失败: {err}"),
                        );
                        self.set_status("⚠️ 网页翻译失败");
                    }
                }
            }
            UiEvent::AiTranslateDelta { gen, delta } => {
                if gen != self.translate_gen {
                    return;
                }
                let current = get_text_view_string(&self.translate_ui.ai_tv);
                if current == "正在请求 AI 流式译文…" {
                    set_text_view_string(&self.translate_ui.ai_tv, &delta);
                } else {
                    append_text_view_string(&self.translate_ui.ai_tv, &delta, true);
                }
            }
            UiEvent::AiTranslateFinished { gen, error } => {
                if gen != self.translate_gen {
                    return;
                }
                if let Some(err) = error {
                    set_text_view_string(&self.translate_ui.ai_tv, &format!("⚠️ {err}"));
                } else {
                    self.set_status("✓ 网页与 AI 翻译全部完成");
                }
            }
            UiEvent::OcrCompleted { outcome } => match outcome {
                Ok(OcrOutcome::Text(text)) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_string(&self.translate_ui.input_tv, &text);
                    self.last_observed_source = text;
                    self.start_translation();
                    self.set_status("✓ 截图 OCR 识别成功，正在翻译…");
                }
                Ok(OcrOutcome::Cancelled) => {
                    self.set_status("已取消截图");
                }
                Ok(OcrOutcome::EmptyText) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_string(
                        &self.translate_ui.web_tv,
                        "⚠️ 所选截图区域未识别到任何文字，请重新框选包含清晰文本的区域。",
                    );
                    self.set_status("⚠️ 截图未识别出文字");
                }
                Ok(OcrOutcome::PermissionDenied(msg)) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_string(&self.translate_ui.web_tv, &format!("🔒 {msg}"));
                    self.set_status("🔒 缺少 macOS 录屏权限");
                }
                Ok(OcrOutcome::Busy) => {
                    self.set_status("正在进行截图框选…");
                }
                Err(err) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_string(
                        &self.translate_ui.web_tv,
                        &format!("❌ OCR 错误: {err}"),
                    );
                    self.set_status("❌ OCR 失败");
                }
            },
            UiEvent::SelectionCapturedForTranslate {
                text,
                fallback_toggle,
            } => match text {
                Some(selected) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_string(&self.translate_ui.input_tv, &selected);
                    self.last_observed_source = selected;
                    self.start_translation();
                }
                None if fallback_toggle => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                }
                None => {}
            },
            UiEvent::SelectionCapturedForChat {
                text,
                fallback_toggle,
            } => match text {
                Some(selected) => {
                    self.switch_tab(TabMode::Chat);
                    self.show_and_focus_window();
                    let _ = self.history.start_new_conversation();
                    self.active_conversation_id = None;
                    self.current_messages.clear();
                    self.send_chat_text(selected);
                }
                None if fallback_toggle => {
                    self.switch_tab(TabMode::Chat);
                    self.show_and_focus_window();
                }
                None => {}
            },
            UiEvent::ChatDelta { gen, delta } => {
                if gen != self.chat_gen {
                    return;
                }
                self.streaming_assistant_buffer.push_str(&delta);
                self.render_chat_transcript();
            }
            UiEvent::ChatFinished { gen, error } => {
                if gen != self.chat_gen {
                    return;
                }
                self.chat_streaming = false;
                if let Some(err) = error {
                    self.current_messages
                        .push(ChatMessage::assistant(format!("⚠️ {err}")));
                    self.set_status("⚠️ AI 回复出错");
                } else {
                    let reply = std::mem::take(&mut self.streaming_assistant_buffer);
                    if !reply.trim().is_empty() {
                        let user_msg = self
                            .current_messages
                            .iter()
                            .rev()
                            .find(|m| m.role == "user")
                            .cloned();
                        let assistant_msg = ChatMessage::assistant(reply);
                        self.current_messages.push(assistant_msg.clone());
                        if let Some(u) = user_msg {
                            let _ = self
                                .history
                                .append_round(self.active_conversation_id, &[u, assistant_msg]);
                            self.refresh_conversations_popup();
                            if self.active_conversation_id.is_none() {
                                self.active_conversation_id =
                                    self.conversations.last().map(|c| c.id);
                            }
                        }
                    }
                    self.set_status("✓ AI 回复完成");
                }
                self.render_chat_transcript();
            }
            UiEvent::ModelsDiscovered { for_chat, result } => match result {
                Ok(models) => {
                    let popup = if for_chat {
                        &self.settings_ui.chat_discovered_popup
                    } else {
                        &self.settings_ui.ai_discovered_popup
                    };
                    let field = if for_chat {
                        &self.settings_ui.chat_model_field
                    } else {
                        &self.settings_ui.ai_model_field
                    };
                    popup.removeAllItems();
                    for m in &models {
                        popup.addItemWithTitle(&NSString::from_str(m));
                    }
                    if let Some(first) = models.first() {
                        set_field_string(field, first);
                    }
                    self.set_status(&format!("✓ 成功探测到 {} 个本地模型", models.len()));
                }
                Err(err) => {
                    self.set_status(&format!("⚠️ {err}"));
                }
            },
        }
    }

    pub fn poll_tick(&mut self) {
        // 1. 监控输入框文本变化并执行 420ms 防抖自动翻译
        if self.window.isVisible() && self.active_tab == TabMode::Translate {
            let current_src = get_text_view_string(&self.translate_ui.input_tv);
            if current_src != self.last_observed_source {
                self.last_observed_source = current_src;
                self.pending_debounce_since = Some(Instant::now());
            }
            if let Some(since) = self.pending_debounce_since {
                if since.elapsed() >= AUTO_TRANSLATE_DEBOUNCE {
                    self.pending_debounce_since = None;
                    let trimmed = self.last_observed_source.trim().to_string();
                    if trimmed != self.last_translated_source {
                        self.start_translation();
                    }
                }
            }
        }

        // 2. 窗口尺寸变更防抖持久化
        if let Some(since) = self.pending_resize_save {
            if since.elapsed() >= RESIZE_SAVE_DEBOUNCE {
                self.pending_resize_save = None;
                let (w, h) = self.last_known_size;
                self.settings.save_window_size(w, h);
            }
        }
    }

    pub fn on_window_did_resign_key(&mut self) {
        if !self.is_pinned && self.active_tab != TabMode::Settings && !self.is_modal_open {
            self.hide_window();
        }
    }

    pub fn on_window_did_resize(&mut self) {
        let Some(content_view) = self.window.contentView() else {
            return;
        };
        let bounds = content_view.bounds();
        let w = bounds.size.width.max(WINDOW_MIN_WIDTH);
        let h = bounds.size.height.max(WINDOW_MIN_HEIGHT);
        if (w - self.last_known_size.0).abs() < 0.5 && (h - self.last_known_size.1).abs() < 0.5 {
            return;
        }
        self.last_known_size = (w, h);
        self.pending_resize_save = Some(Instant::now());

        self.effect_view.setFrame(bounds);
        self.tab_translate_btn
            .setFrame(top_rect(h, 14.0, 10.0, 74.0, 28.0));
        self.tab_chat_btn
            .setFrame(top_rect(h, 92.0, 10.0, 86.0, 28.0));
        self.tab_settings_btn
            .setFrame(top_rect(h, 182.0, 10.0, 72.0, 28.0));
        self.status_label
            .setFrame(top_rect(h, 264.0, 14.0, (w - 445.0).max(120.0), 20.0));
        self.ocr_btn
            .setFrame(top_rect(h, w - 172.0, 10.0, 86.0, 28.0));
        self.pin_btn
            .setFrame(top_rect(h, w - 82.0, 10.0, 68.0, 28.0));

        let body_h = (h - 46.0).max(320.0);
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, body_h));

        // Relayout Translate view
        self.translate_ui.root.setFrame(body_frame);
        self.translate_ui
            .source_popup
            .setFrame(top_rect(body_h, 14.0, 6.0, 145.0, 26.0));
        self.translate_ui
            .swap_btn
            .setFrame(top_rect(body_h, 163.0, 6.0, 38.0, 26.0));
        self.translate_ui
            .target_popup
            .setFrame(top_rect(body_h, 205.0, 6.0, 165.0, 26.0));
        self.translate_ui
            .translate_btn
            .setFrame(top_rect(body_h, w - 166.0, 6.0, 92.0, 26.0));
        self.translate_ui
            .clear_btn
            .setFrame(top_rect(body_h, w - 70.0, 6.0, 56.0, 26.0));

        let input_h = 118.0;
        self.translate_ui
            .input_scroll
            .setFrame(top_rect(body_h, 14.0, 38.0, w - 28.0, input_h));
        let cards_top = 38.0 + input_h + 10.0;
        let cards_h = (body_h - cards_top - 14.0).max(160.0);
        let col_w = ((w - 38.0) / 2.0).max(200.0);

        self.translate_ui
            .web_card
            .setFrame(top_rect(body_h, 14.0, cards_top, col_w, cards_h));
        self.translate_ui
            .web_header_label
            .setFrame(top_rect(cards_h, 10.0, 8.0, col_w - 76.0, 18.0));
        self.translate_ui
            .web_copy_btn
            .setFrame(top_rect(cards_h, col_w - 64.0, 5.0, 54.0, 24.0));
        self.translate_ui
            .web_scroll
            .setFrame(top_rect(cards_h, 4.0, 32.0, col_w - 8.0, cards_h - 36.0));

        self.translate_ui.ai_card.setFrame(top_rect(
            body_h,
            14.0 + col_w + 10.0,
            cards_top,
            col_w,
            cards_h,
        ));
        self.translate_ui
            .ai_header_label
            .setFrame(top_rect(cards_h, 10.0, 8.0, col_w - 76.0, 18.0));
        self.translate_ui
            .ai_copy_btn
            .setFrame(top_rect(cards_h, col_w - 64.0, 5.0, 54.0, 24.0));
        self.translate_ui
            .ai_scroll
            .setFrame(top_rect(cards_h, 4.0, 32.0, col_w - 8.0, cards_h - 36.0));

        // Relayout Chat view
        self.chat_ui.root.setFrame(body_frame);
        self.chat_ui
            .conv_popup
            .setFrame(top_rect(body_h, 14.0, 6.0, 260.0, 26.0));
        self.chat_ui
            .new_conv_btn
            .setFrame(top_rect(body_h, 280.0, 6.0, 82.0, 26.0));
        self.chat_ui
            .clear_hist_btn
            .setFrame(top_rect(body_h, 366.0, 6.0, 82.0, 26.0));
        self.chat_ui
            .stop_btn
            .setFrame(top_rect(body_h, w - 166.0, 6.0, 74.0, 26.0));
        self.chat_ui
            .copy_reply_btn
            .setFrame(top_rect(body_h, w - 88.0, 6.0, 74.0, 26.0));
        let chat_input_h = 72.0;
        let transcript_h = (body_h - 38.0 - chat_input_h - 22.0).max(180.0);
        self.chat_ui
            .transcript_scroll
            .setFrame(top_rect(body_h, 14.0, 38.0, w - 28.0, transcript_h));
        let chat_input_top = 38.0 + transcript_h + 8.0;
        self.chat_ui.input_scroll.setFrame(top_rect(
            body_h,
            14.0,
            chat_input_top,
            w - 104.0,
            chat_input_h,
        ));
        self.chat_ui.send_btn.setFrame(top_rect(
            body_h,
            w - 84.0,
            chat_input_top + 18.0,
            70.0,
            36.0,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_tag_round_trips_all_known_tags() {
        let tags = [
            ActionTag::TabTranslate,
            ActionTag::TabChat,
            ActionTag::TabSettings,
            ActionTag::TriggerOcr,
            ActionTag::TogglePin,
            ActionTag::TrayQuit,
            ActionTag::HideOrEscape,
            ActionTag::TranslateNow,
            ActionTag::ClearTranslate,
            ActionTag::SwapLanguages,
            ActionTag::SourceLangChanged,
            ActionTag::TargetLangChanged,
            ActionTag::CopyWebResult,
            ActionTag::CopyAiResult,
            ActionTag::SendChat,
            ActionTag::NewConversation,
            ActionTag::ClearChatHistory,
            ActionTag::StopChatStream,
            ActionTag::SelectConversation,
            ActionTag::CopyLastReply,
            ActionTag::SaveSettings,
            ActionTag::AiTransBackendChanged,
            ActionTag::AiTransFillDefaultUrl,
            ActionTag::AiTransDiscoverModels,
            ActionTag::AiTransModelPopupSelected,
            ActionTag::AiTransBrowseGguf,
            ActionTag::AiTransBrowseTokenizer,
            ActionTag::AiTransClearKey,
            ActionTag::ChatBackendChanged,
            ActionTag::ChatFillDefaultUrl,
            ActionTag::ChatDiscoverModels,
            ActionTag::ChatModelPopupSelected,
            ActionTag::ChatBrowseGguf,
            ActionTag::ChatCopyFromTrans,
            ActionTag::ChatClearKey,
            ActionTag::ResetDefaultShortcuts,
            ActionTag::OpacitySliderChanged,
        ];
        for tag in tags {
            assert_eq!(ActionTag::from_raw(tag as isize), Some(tag));
        }
        assert_eq!(ActionTag::from_raw(9999), None);
    }

    #[test]
    fn converts_top_origin_rect_to_appkit_bottom_origin() {
        let r = top_rect(500.0, 14.0, 10.0, 80.0, 28.0);
        assert_eq!(r.origin.x, 14.0);
        assert_eq!(r.origin.y, 462.0);
        assert_eq!(r.size.width, 80.0);
        assert_eq!(r.size.height, 28.0);
    }
}

