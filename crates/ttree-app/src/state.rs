//! `ttree-app` 核心控制器：复刻原版极简上下分栏排版（右上角功能区 + 上方翻译输入框 + 语言选择条 + Google/Bing 负载均衡译文框 + AI 译文框）。

#![cfg(target_os = "macos")]
#![allow(clippy::too_many_arguments)]

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
    AppearanceConfig, CancelSlot, ChatAiConfig, ChatMessage, Conversation, PaneLayoutConfig,
    ShortcutConfig, ADAPTIVE_TARGET, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN, SOURCE_LANGUAGES,
    TARGET_LANGUAGES, TOP_BAR_HEIGHT, WINDOW_MAX_HEIGHT, WINDOW_MAX_WIDTH, WINDOW_MIN_HEIGHT,
    WINDOW_MIN_WIDTH,
};
use ttree_ocr::OcrOutcome;
use ttree_platform_macos::hotkey::{parse_shortcut_config, ParsedShortcuts, ShortcutAction};
use ttree_storage::{HistoryStore, SettingsStore};

use crate::ui_helpers::{
    add_subview, append_text_view_string, copy_to_pasteboard, get_field_string,
    get_text_view_string, make_button, make_checkbox, make_divider, make_flat_button, make_label,
    make_popup, make_scrollable_text_view, make_secure_field, make_slider, make_split_bar,
    make_surface_box, make_symbol_button, make_text_field, set_button_symbol, set_field_string,
    set_flat_button_active, set_split_bar_frame, set_text_view_string, set_text_view_styled,
    top_rect, SplitBarWidget, SPLITTER_HIT_HEIGHT,
};

const AUTO_TRANSLATE_DEBOUNCE: Duration = Duration::from_millis(650);
const RESIZE_SAVE_DEBOUNCE: Duration = Duration::from_millis(350);

const SPLITTER_SOURCE: u8 = 1;
const SPLITTER_LANG: u8 = 2;
const SPLITTER_WEB: u8 = 3;
const SPLITTER_CHAT: u8 = 4;

const WEB_PLACEHOLDER: &str = "译文显示在这里";
const AI_PLACEHOLDER: &str = "AI 译文显示在这里";
const WEB_LOADING_TEXT: &str = "翻译中…";
const AI_LOADING_TEXT: &str = "AI 翻译中…";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabMode {
    Translate,
    Chat,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Translate,
    Chat,
    Shortcuts,
    General,
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
    CloseSettings = 8,

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
    ToggleHistoryBar = 26,

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
    AutostartToggled = 52,

    SettingsSecTranslate = 60,
    SettingsSecChat = 61,
    SettingsSecShortcuts = 62,
    SettingsSecGeneral = 63,
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
            8 => Some(Self::CloseSettings),
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
            26 => Some(Self::ToggleHistoryBar),
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
            52 => Some(Self::AutostartToggled),
            60 => Some(Self::SettingsSecTranslate),
            61 => Some(Self::SettingsSecChat),
            62 => Some(Self::SettingsSecShortcuts),
            63 => Some(Self::SettingsSecGeneral),
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

/// 翻译视图（原版四层上下分栏：上方原文输入框 → 语言选择条 → 谷歌/微软译文框 → AI 译文框，每栏间均可上下拖拽调整高度）
pub struct TranslateViewWidgets {
    pub root: Retained<NSView>,
    pub source_surface: Retained<NSBox>,
    pub input_scroll: Retained<NSScrollView>,
    pub input_tv: Retained<NSTextView>,
    pub input_placeholder: Retained<NSTextField>,
    pub source_split_bar: SplitBarWidget,

    pub lang_surface: Retained<NSBox>,
    pub source_popup: Retained<NSPopUpButton>,
    pub swap_btn: Retained<NSButton>,
    pub target_popup: Retained<NSPopUpButton>,
    pub status_label: Retained<NSTextField>,
    pub lang_split_bar: SplitBarWidget,

    pub web_header_label: Retained<NSTextField>,
    pub web_copy_btn: Retained<NSButton>,
    pub web_scroll: Retained<NSScrollView>,
    pub web_tv: Retained<NSTextView>,
    pub web_split_bar: SplitBarWidget,

    pub ai_header_label: Retained<NSTextField>,
    pub ai_copy_btn: Retained<NSButton>,
    pub ai_scroll: Retained<NSScrollView>,
    pub ai_tv: Retained<NSTextView>,
}

pub struct ChatViewWidgets {
    pub root: Retained<NSView>,
    pub history_bar: Retained<NSView>,
    pub conv_popup: Retained<NSPopUpButton>,
    pub clear_hist_btn: Retained<NSButton>,
    pub history_bottom_line: Retained<NSBox>,
    pub transcript_scroll: Retained<NSScrollView>,
    pub transcript_tv: Retained<NSTextView>,
    pub composer_split_bar: SplitBarWidget,
    pub composer_surface: Retained<NSBox>,
    pub input_scroll: Retained<NSScrollView>,
    pub input_tv: Retained<NSTextView>,
    pub input_placeholder: Retained<NSTextField>,
    pub send_btn: Retained<NSButton>,
}

pub struct SettingsViewWidgets {
    pub root: Retained<NSView>,
    pub top_divider: Retained<NSBox>,
    pub sidebar_divider: Retained<NSBox>,
    pub nav_translate_btn: Retained<NSButton>,
    pub nav_chat_btn: Retained<NSButton>,
    pub nav_shortcuts_btn: Retained<NSButton>,
    pub nav_general_btn: Retained<NSButton>,

    pub sec_translate: Retained<NSView>,
    pub sec_chat: Retained<NSView>,
    pub sec_shortcuts: Retained<NSView>,
    pub sec_general: Retained<NSView>,

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
    pub settings_status_label: Retained<NSTextField>,
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

    // Top Header Bar (EngineTabs on Left, Toolbar Actions on Top-Right)
    pub segmented_bg: Retained<NSBox>,
    pub segmented_pill: Retained<NSBox>,
    pub tab_translate_btn: Retained<NSButton>,
    pub tab_chat_btn: Retained<NSButton>,
    pub settings_title_label: Retained<NSTextField>,

    pub pin_btn: Retained<NSButton>,
    pub new_chat_top_btn: Retained<NSButton>,
    pub history_top_btn: Retained<NSButton>,
    pub gear_btn: Retained<NSButton>,
    pub done_settings_btn: Retained<NSButton>,

    pub translate_ui: TranslateViewWidgets,
    pub chat_ui: ChatViewWidgets,
    pub settings_ui: SettingsViewWidgets,

    pub active_tab: TabMode,
    pub last_main_mode: TabMode,
    pub active_settings_sec: SettingsSection,
    pub show_chat_history_bar: bool,
    pub is_pinned: bool,
    pub pin_before_settings: bool,
    pub is_modal_open: bool,

    pub translate_cancel: CancelSlot,
    pub translate_gen: u64,
    pub last_observed_source: String,
    pub last_translated_source: String,
    pub pending_debounce_since: Option<Instant>,
    pub web_has_content: bool,
    pub ai_has_content: bool,

    pub chat_cancel: CancelSlot,
    pub chat_gen: u64,
    pub chat_streaming: bool,
    pub conversations: Vec<Conversation>,
    pub active_conversation_id: Option<usize>,
    pub current_messages: Vec<ChatMessage>,
    pub streaming_assistant_buffer: String,

    pub pending_clear_ai_key: bool,
    pub pending_clear_chat_key: bool,

    pub pane_layout: PaneLayoutConfig,
    pub is_programmatic_resize: bool,
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
        let pane_layout = settings.pane_layout();
        let ai_enabled = settings.ai_translation_for_request().enabled;
        let win_w = pane_layout.width;
        let win_h = pane_layout.translate_window_height(ai_enabled);

        // ── 顶部栏：左上角 [翻译 | AI] 分段切换器，右上角图标功能区（全淡色） ──
        let segmented_bg =
            make_surface_box(mtm, top_rect(win_h, 12.0, 5.0, 128.0, 28.0), 8.0, 0.35);
        let segmented_pill =
            make_surface_box(mtm, top_rect(win_h, 15.0, 7.0, 59.0, 24.0), 6.0, 0.70);
        let tab_translate_btn = make_flat_button(
            mtm,
            top_rect(win_h, 15.0, 7.0, 59.0, 24.0),
            "翻译",
            12.5,
            true,
            false,
            ActionTag::TabTranslate as isize,
            dispatcher,
            action_sel,
        );
        let tab_chat_btn = make_flat_button(
            mtm,
            top_rect(win_h, 78.0, 7.0, 59.0, 24.0),
            "AI",
            12.5,
            true,
            false,
            ActionTag::TabChat as isize,
            dispatcher,
            action_sel,
        );
        set_flat_button_active(&tab_translate_btn, true, false);
        set_flat_button_active(&tab_chat_btn, false, false);

        let settings_title_label = make_label(
            mtm,
            top_rect(win_h, 16.0, 8.0, 120.0, 22.0),
            "设置",
            15.0,
            true,
            false,
        );
        settings_title_label.setHidden(true);

        // 右上角功能区按钮
        let pin_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 76.0, 5.0, 28.0, 28.0),
            "pin",
            "📌",
            "固定窗口",
            ActionTag::TogglePin as isize,
            dispatcher,
            action_sel,
        );
        let new_chat_top_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 108.0, 5.0, 28.0, 28.0),
            "square.and.pencil",
            "＋",
            "新对话",
            ActionTag::NewConversation as isize,
            dispatcher,
            action_sel,
        );
        new_chat_top_btn.setHidden(true);

        let history_top_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 76.0, 5.0, 28.0, 28.0),
            "clock.arrow.circlepath",
            "⏱",
            "查看对话历史",
            ActionTag::ToggleHistoryBar as isize,
            dispatcher,
            action_sel,
        );
        history_top_btn.setHidden(true);

        let gear_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 42.0, 5.0, 28.0, 28.0),
            "gearshape",
            "⚙",
            "设置",
            ActionTag::TabSettings as isize,
            dispatcher,
            action_sel,
        );

        let done_settings_btn = make_flat_button(
            mtm,
            top_rect(win_h, win_w - 56.0, 6.0, 44.0, 26.0),
            "完成",
            13.0,
            true,
            false,
            ActionTag::CloseSettings as isize,
            dispatcher,
            action_sel,
        );
        done_settings_btn.setHidden(true);

        add_subview(&effect_view, &segmented_bg);
        add_subview(&effect_view, &segmented_pill);
        add_subview(&effect_view, &tab_translate_btn);
        add_subview(&effect_view, &tab_chat_btn);
        add_subview(&effect_view, &settings_title_label);
        add_subview(&effect_view, &pin_btn);
        add_subview(&effect_view, &new_chat_top_btn);
        add_subview(&effect_view, &history_top_btn);
        add_subview(&effect_view, &gear_btn);
        add_subview(&effect_view, &done_settings_btn);

        let body_h = (win_h - TOP_BAR_HEIGHT).max(174.0);
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(win_w, body_h));

        let translate_ui = Self::build_translate_view(
            mtm,
            body_frame,
            &pane_layout,
            ai_enabled,
            dispatcher,
            action_sel,
        );
        let chat_ui =
            Self::build_chat_view(mtm, body_frame, &pane_layout, dispatcher, action_sel);
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
            segmented_bg,
            segmented_pill,
            tab_translate_btn,
            tab_chat_btn,
            settings_title_label,
            pin_btn,
            new_chat_top_btn,
            history_top_btn,
            gear_btn,
            done_settings_btn,
            translate_ui,
            chat_ui,
            settings_ui,
            active_tab: TabMode::Translate,
            last_main_mode: TabMode::Translate,
            active_settings_sec: SettingsSection::Translate,
            show_chat_history_bar: false,
            is_pinned: false,
            pin_before_settings: false,
            is_modal_open: false,
            translate_cancel: CancelSlot::new(),
            translate_gen: 0,
            last_observed_source: String::new(),
            last_translated_source: String::new(),
            pending_debounce_since: None,
            web_has_content: false,
            ai_has_content: false,
            chat_cancel: CancelSlot::new(),
            chat_gen: 0,
            chat_streaming: false,
            conversations,
            active_conversation_id,
            current_messages,
            streaming_assistant_buffer: String::new(),
            pending_clear_ai_key: false,
            pending_clear_chat_key: false,
            pane_layout,
            is_programmatic_resize: false,
            last_known_size: (win_w, win_h),
            pending_resize_save: None,
        };

        ctrl.update_window_min_max_constraints();
        ctrl.refresh_ai_header_badge();
        ctrl.refresh_conversations_popup();
        ctrl.render_chat_transcript();
        ctrl
    }

    /// 构建原版四层上下分栏翻译面板（每一栏均带独立最小/最大高度限制与可拖拽分栏条）：
    /// 1. 上方翻译输入框 (`source_surface` + `input_tv`)
    /// 2. 语言选择条 (`lang_surface`: `[自动检测] → [中 ⇄ 英（自适应）]`)
    /// 3. 谷歌/微软负载均衡翻译显示框 (`web_header_label` + `web_tv`)
    /// 4. AI 翻译显示框 (`ai_header_label` + `ai_tv`)
    fn build_translate_view(
        mtm: MainThreadMarker,
        frame: NSRect,
        layout: &PaneLayoutConfig,
        ai_enabled: bool,
        dispatcher: &AnyObject,
        action_sel: Sel,
    ) -> TranslateViewWidgets {
        let w = frame.size.width;
        let h = frame.size.height;
        let root = NSView::initWithFrame(mtm.alloc(), frame);

        let source_h = layout.source_height;
        let lang_h = layout.lang_height;
        let results_top = source_h + lang_h;
        let web_h = layout.web_height;
        let ai_top = results_top + web_h;
        let ai_h = layout.ai_height;

        // 1. 上方翻译输入框
        let source_surface =
            make_surface_box(mtm, top_rect(h, 0.0, 0.0, w, source_h), 0.0, 0.25);
        let (input_scroll, input_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 2.0, 2.0, w - 4.0, (source_h - 6.0).max(40.0)),
            true,
            15.0,
            Some(dispatcher),
        );
        let input_placeholder = make_label(
            mtm,
            top_rect(h, 16.0, 12.0, w - 32.0, 20.0),
            "输入文字，回车翻译",
            15.0,
            false,
            true,
        );
        let source_split_bar = make_split_bar(
            mtm,
            top_rect(
                h,
                0.0,
                source_h - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
            SPLITTER_SOURCE,
        );

        // 2. 下方选择语言条
        let lang_surface =
            make_surface_box(mtm, top_rect(h, 0.0, source_h, w, lang_h), 0.0, 0.35);
        let src_labels: Vec<&str> = SOURCE_LANGUAGES.iter().map(|l| l.label).collect();
        let tgt_labels: Vec<&str> = TARGET_LANGUAGES.iter().map(|l| l.label).collect();
        let popup_top = source_h + ((lang_h - 24.0) * 0.5).max(2.0);

        let source_popup = make_popup(
            mtm,
            top_rect(h, 12.0, popup_top, 114.0, 24.0),
            &src_labels,
            0,
            false,
            ActionTag::SourceLangChanged as isize,
            dispatcher,
            action_sel,
        );
        let swap_btn = make_flat_button(
            mtm,
            top_rect(h, 128.0, popup_top, 24.0, 24.0),
            "→",
            12.5,
            false,
            false,
            ActionTag::SwapLanguages as isize,
            dispatcher,
            action_sel,
        );
        let target_popup = make_popup(
            mtm,
            top_rect(h, 154.0, popup_top, 152.0, 24.0),
            &tgt_labels,
            0,
            false,
            ActionTag::TargetLangChanged as isize,
            dispatcher,
            action_sel,
        );
        let status_label = make_label(
            mtm,
            top_rect(h, 310.0, popup_top + 3.0, (w - 322.0).max(60.0), 18.0),
            "",
            11.0,
            false,
            true,
        );
        let lang_split_bar = make_split_bar(
            mtm,
            top_rect(
                h,
                0.0,
                results_top - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
            SPLITTER_LANG,
        );

        // 3. 接下方谷歌/微软负载均衡翻译显示框
        let web_header_label = make_label(
            mtm,
            top_rect(h, 16.0, results_top + 7.0, 240.0, 15.0),
            "翻译  ·  GOOGLE / BING",
            10.5,
            false,
            true,
        );
        let web_copy_btn = make_flat_button(
            mtm,
            top_rect(h, w - 58.0, results_top + 4.0, 46.0, 20.0),
            "复制",
            11.0,
            false,
            false,
            ActionTag::CopyWebResult as isize,
            dispatcher,
            action_sel,
        );
        web_copy_btn.setHidden(true);

        let (web_scroll, web_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 2.0, results_top + 22.0, w - 4.0, (web_h - 26.0).max(40.0)),
            false,
            14.5,
            None,
        );
        set_text_view_styled(&web_tv, WEB_PLACEHOLDER, true);
        let web_split_bar = make_split_bar(
            mtm,
            top_rect(
                h,
                0.0,
                results_top + web_h - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
            SPLITTER_WEB,
        );

        // 4. 接下方 AI 翻译显示框
        let ai_header_label = make_label(
            mtm,
            top_rect(h, 16.0, ai_top + 7.0, 280.0, 15.0),
            "AI",
            10.5,
            false,
            true,
        );
        let ai_copy_btn = make_flat_button(
            mtm,
            top_rect(h, w - 58.0, ai_top + 4.0, 46.0, 20.0),
            "复制",
            11.0,
            false,
            false,
            ActionTag::CopyAiResult as isize,
            dispatcher,
            action_sel,
        );
        ai_copy_btn.setHidden(true);

        let (ai_scroll, ai_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 2.0, ai_top + 22.0, w - 4.0, (ai_h - 26.0).max(40.0)),
            false,
            14.5,
            None,
        );
        set_text_view_styled(&ai_tv, AI_PLACEHOLDER, true);

        web_split_bar.handle.setHidden(!ai_enabled);
        ai_header_label.setHidden(!ai_enabled);
        ai_scroll.setHidden(!ai_enabled);

        add_subview(&root, &source_surface);
        add_subview(&root, &input_placeholder);
        add_subview(&root, &input_scroll);

        add_subview(&root, &lang_surface);
        add_subview(&root, &source_popup);
        add_subview(&root, &swap_btn);
        add_subview(&root, &target_popup);
        add_subview(&root, &status_label);

        add_subview(&root, &web_header_label);
        add_subview(&root, &web_copy_btn);
        add_subview(&root, &web_scroll);

        add_subview(&root, &ai_header_label);
        add_subview(&root, &ai_copy_btn);
        add_subview(&root, &ai_scroll);

        // 将可拖拽分栏条放在最顶层以确保鼠标悬停与拖拽命中
        add_subview(&root, &source_split_bar.handle);
        add_subview(&root, &lang_split_bar.handle);
        add_subview(&root, &web_split_bar.handle);

        TranslateViewWidgets {
            root,
            source_surface,
            input_scroll,
            input_tv,
            input_placeholder,
            source_split_bar,
            lang_surface,
            source_popup,
            swap_btn,
            target_popup,
            status_label,
            lang_split_bar,
            web_header_label,
            web_copy_btn,
            web_scroll,
            web_tv,
            web_split_bar,
            ai_header_label,
            ai_copy_btn,
            ai_scroll,
            ai_tv,
        }
    }

    fn build_chat_view(
        mtm: MainThreadMarker,
        frame: NSRect,
        layout: &PaneLayoutConfig,
        dispatcher: &AnyObject,
        action_sel: Sel,
    ) -> ChatViewWidgets {
        let w = frame.size.width;
        let h = frame.size.height;
        let root = NSView::initWithFrame(mtm.alloc(), frame);

        // 顶部折叠式历史对话选择条（点击右上角时钟按钮展开/收起）
        let history_bar = NSView::initWithFrame(mtm.alloc(), top_rect(h, 0.0, 0.0, w, 34.0));
        let conv_popup = make_popup(
            mtm,
            top_rect(34.0, 12.0, 4.0, w - 104.0, 24.0),
            &["当前对话"],
            0,
            true,
            ActionTag::SelectConversation as isize,
            dispatcher,
            action_sel,
        );
        let clear_hist_btn = make_flat_button(
            mtm,
            top_rect(34.0, w - 84.0, 4.0, 72.0, 24.0),
            "清空历史",
            11.5,
            false,
            false,
            ActionTag::ClearChatHistory as isize,
            dispatcher,
            action_sel,
        );
        let history_bottom_line = make_divider(mtm, top_rect(34.0, 0.0, 33.0, w, 1.0));
        add_subview(&history_bar, &conv_popup);
        add_subview(&history_bar, &clear_hist_btn);
        add_subview(&history_bar, &history_bottom_line);
        history_bar.setHidden(true);

        let composer_h = layout.chat_input_height;
        let transcript_h = (h - composer_h).max(120.0);
        let (transcript_scroll, transcript_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 4.0, 0.0, w - 8.0, transcript_h),
            false,
            14.0,
            None,
        );

        let composer_top = transcript_h;
        let composer_split_bar = make_split_bar(
            mtm,
            top_rect(
                h,
                0.0,
                composer_top - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
            SPLITTER_CHAT,
        );
        let composer_surface = make_surface_box(
            mtm,
            top_rect(h, 0.0, composer_top + 1.0, w, composer_h - 1.0),
            0.0,
            0.35,
        );
        let (input_scroll, input_tv) = make_scrollable_text_view(
            mtm,
            top_rect(h, 4.0, composer_top + 6.0, w - 84.0, (composer_h - 12.0).max(40.0)),
            true,
            14.0,
            Some(dispatcher),
        );
        let input_placeholder = make_label(
            mtm,
            top_rect(h, 18.0, composer_top + 15.0, 200.0, 20.0),
            "输入消息，回车发送",
            14.0,
            false,
            true,
        );
        let send_btn = make_button(
            mtm,
            top_rect(h, w - 74.0, composer_top + (composer_h - 28.0) * 0.5, 62.0, 28.0),
            "发送",
            ActionTag::SendChat as isize,
            dispatcher,
            action_sel,
        );

        add_subview(&root, &transcript_scroll);
        add_subview(&root, &history_bar);
        add_subview(&root, &composer_surface);
        add_subview(&root, &input_placeholder);
        add_subview(&root, &input_scroll);
        add_subview(&root, &send_btn);
        add_subview(&root, &composer_split_bar.handle);

        ChatViewWidgets {
            root,
            history_bar,
            conv_popup,
            clear_hist_btn,
            history_bottom_line,
            transcript_scroll,
            transcript_tv,
            composer_split_bar,
            composer_surface,
            input_scroll,
            input_tv,
            input_placeholder,
            send_btn,
        }
    }

    /// 构建原版左侧分类栏 (`AI 翻译` / `AI 对话` / `快捷键` / `通用`) + 右侧纵向表单的设置面板。
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

        let top_divider = make_divider(mtm, top_rect(h, 12.0, 0.0, w - 24.0, 1.0));
        let sidebar_w = 114.0;
        let sidebar_divider = make_divider(mtm, top_rect(h, sidebar_w, 1.0, 1.0, h - 1.0));

        let nav_translate_btn = make_flat_button(
            mtm,
            top_rect(h, 10.0, 12.0, sidebar_w - 20.0, 30.0),
            "● AI 翻译",
            13.0,
            true,
            true,
            ActionTag::SettingsSecTranslate as isize,
            dispatcher,
            action_sel,
        );
        let nav_chat_btn = make_flat_button(
            mtm,
            top_rect(h, 10.0, 46.0, sidebar_w - 20.0, 30.0),
            "   AI 对话",
            13.0,
            false,
            false,
            ActionTag::SettingsSecChat as isize,
            dispatcher,
            action_sel,
        );
        let nav_shortcuts_btn = make_flat_button(
            mtm,
            top_rect(h, 10.0, 80.0, sidebar_w - 20.0, 30.0),
            "   快捷键",
            13.0,
            false,
            false,
            ActionTag::SettingsSecShortcuts as isize,
            dispatcher,
            action_sel,
        );
        let nav_general_btn = make_flat_button(
            mtm,
            top_rect(h, 10.0, 114.0, sidebar_w - 20.0, 30.0),
            "   通用",
            13.0,
            false,
            false,
            ActionTag::SettingsSecGeneral as isize,
            dispatcher,
            action_sel,
        );

        add_subview(&root, &top_divider);
        add_subview(&root, &sidebar_divider);
        add_subview(&root, &nav_translate_btn);
        add_subview(&root, &nav_chat_btn);
        add_subview(&root, &nav_shortcuts_btn);
        add_subview(&root, &nav_general_btn);

        let content_x = sidebar_w + 14.0;
        let content_w = (w - content_x - 14.0).max(310.0);
        let sec_frame = top_rect(h, content_x, 6.0, content_w, h - 38.0);
        let sec_h = sec_frame.size.height;

        let sec_translate = NSView::initWithFrame(mtm.alloc(), sec_frame);
        let sec_chat = NSView::initWithFrame(mtm.alloc(), sec_frame);
        let sec_shortcuts = NSView::initWithFrame(mtm.alloc(), sec_frame);
        let sec_general = NSView::initWithFrame(mtm.alloc(), sec_frame);

        sec_chat.setHidden(true);
        sec_shortcuts.setHidden(true);
        sec_general.setHidden(true);

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

        // ── Section 1: AI 翻译 ──
        let t_head = make_label(
            mtm,
            top_rect(sec_h, 0.0, 6.0, 180.0, 18.0),
            "AI 翻译（云端 / 本地服务 / GGUF）",
            12.0,
            true,
            true,
        );
        let ai_enabled_chk = make_checkbox(
            mtm,
            top_rect(sec_h, content_w - 110.0, 4.0, 110.0, 20.0),
            "启用 AI 翻译",
            ai_cfg.enabled,
            0,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &t_head);
        add_subview(&sec_translate, &ai_enabled_chk);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 32.0, 72.0, 18.0), "接入方式", 12.0, false, true),
        );
        let ai_backend_popup = make_popup(
            mtm,
            top_rect(sec_h, 74.0, 29.0, content_w - 144.0, 25.0),
            &backend_labels,
            ai_backend_idx,
            true,
            ActionTag::AiTransBackendChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_ai_def_url = make_button(
            mtm,
            top_rect(sec_h, content_w - 66.0, 29.0, 66.0, 25.0),
            "默认地址",
            ActionTag::AiTransFillDefaultUrl as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &ai_backend_popup);
        add_subview(&sec_translate, &btn_ai_def_url);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 64.0, 72.0, 18.0), "接口地址", 12.0, false, true),
        );
        let ai_base_url_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 62.0, content_w - 74.0, 23.0),
            &ai_cfg.base_url,
            "https://api.openai.com/v1 或 http://127.0.0.1:11434",
        );
        add_subview(&sec_translate, &ai_base_url_field);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 96.0, 72.0, 18.0), "API 密钥", 12.0, false, true),
        );
        let ai_key_field = make_secure_field(
            mtm,
            top_rect(sec_h, 74.0, 94.0, content_w - 190.0, 23.0),
            if ai_cfg.has_api_key {
                "已保存，留空保持不变"
            } else {
                "sk-… (本地服务/GGUF免填)"
            },
        );
        let ai_key_status_label = make_label(
            mtm,
            top_rect(sec_h, content_w - 112.0, 96.0, 50.0, 18.0),
            if ai_cfg.has_api_key { "已保存" } else { "未设置" },
            11.0,
            false,
            true,
        );
        let btn_ai_clr_key = make_button(
            mtm,
            top_rect(sec_h, content_w - 60.0, 93.0, 60.0, 24.0),
            "清除",
            ActionTag::AiTransClearKey as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &ai_key_field);
        add_subview(&sec_translate, &ai_key_status_label);
        add_subview(&sec_translate, &btn_ai_clr_key);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 128.0, 72.0, 18.0), "模型名称", 12.0, false, true),
        );
        let ai_model_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 126.0, content_w - 216.0, 23.0),
            &ai_cfg.model,
            "gpt-4o-mini / qwen2.5:7b",
        );
        let btn_ai_discover = make_button(
            mtm,
            top_rect(sec_h, content_w - 138.0, 125.0, 66.0, 25.0),
            "探测模型",
            ActionTag::AiTransDiscoverModels as isize,
            dispatcher,
            action_sel,
        );
        let ai_discovered_popup = make_popup(
            mtm,
            top_rect(sec_h, content_w - 70.0, 125.0, 70.0, 25.0),
            &["选择…"],
            0,
            true,
            ActionTag::AiTransModelPopupSelected as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &ai_model_field);
        add_subview(&sec_translate, &btn_ai_discover);
        add_subview(&sec_translate, &ai_discovered_popup);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 160.0, 72.0, 18.0), "GGUF模型", 12.0, false, true),
        );
        let ai_gguf_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 158.0, content_w - 138.0, 23.0),
            &ai_cfg.gguf_model_path,
            "本地 .gguf 模型文件路径（仅直接加载模式需填）",
        );
        let btn_ai_gguf = make_button(
            mtm,
            top_rect(sec_h, content_w - 60.0, 157.0, 60.0, 25.0),
            "浏览…",
            ActionTag::AiTransBrowseGguf as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &ai_gguf_field);
        add_subview(&sec_translate, &btn_ai_gguf);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 192.0, 72.0, 18.0), "分词词表", 12.0, false, true),
        );
        let ai_tokenizer_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 190.0, content_w - 138.0, 23.0),
            &ai_cfg.gguf_tokenizer_path,
            "可选 tokenizer.json (留空自动读取 GGUF 内嵌词表)",
        );
        let btn_ai_tok = make_button(
            mtm,
            top_rect(sec_h, content_w - 60.0, 189.0, 60.0, 25.0),
            "浏览…",
            ActionTag::AiTransBrowseTokenizer as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &ai_tokenizer_field);
        add_subview(&sec_translate, &btn_ai_tok);

        add_subview(
            &sec_translate,
            &make_label(mtm, top_rect(sec_h, 0.0, 224.0, 72.0, 18.0), "翻译提示词", 12.0, false, true),
        );
        let ai_prompt_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 222.0, content_w - 74.0, 23.0),
            &ai_cfg.translate_prompt,
            "用 {source} 和 {target} 表示源语言和目标语言",
        );
        add_subview(&sec_translate, &ai_prompt_field);

        let save_ai_btn = make_button(
            mtm,
            top_rect(sec_h, 0.0, 260.0, 96.0, 28.0),
            "保存配置",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_translate, &save_ai_btn);

        // ── Section 2: AI 对话 ──
        let c_head = make_label(
            mtm,
            top_rect(sec_h, 0.0, 6.0, 180.0, 18.0),
            "AI 对话（独立配置）",
            12.0,
            true,
            true,
        );
        let btn_sync_from_trans = make_button(
            mtm,
            top_rect(sec_h, content_w - 138.0, 3.0, 138.0, 24.0),
            "复用 AI 翻译配置",
            ActionTag::ChatCopyFromTrans as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_chat, &c_head);
        add_subview(&sec_chat, &btn_sync_from_trans);

        add_subview(
            &sec_chat,
            &make_label(mtm, top_rect(sec_h, 0.0, 36.0, 72.0, 18.0), "接入方式", 12.0, false, true),
        );
        let chat_backend_popup = make_popup(
            mtm,
            top_rect(sec_h, 74.0, 33.0, content_w - 144.0, 25.0),
            &backend_labels,
            chat_backend_idx,
            true,
            ActionTag::ChatBackendChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_chat_def_url = make_button(
            mtm,
            top_rect(sec_h, content_w - 66.0, 33.0, 66.0, 25.0),
            "默认地址",
            ActionTag::ChatFillDefaultUrl as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_chat, &chat_backend_popup);
        add_subview(&sec_chat, &btn_chat_def_url);

        add_subview(
            &sec_chat,
            &make_label(mtm, top_rect(sec_h, 0.0, 68.0, 72.0, 18.0), "接口地址", 12.0, false, true),
        );
        let chat_base_url_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 66.0, content_w - 74.0, 23.0),
            &chat_cfg.base_url,
            "https://api.openai.com/v1 或 http://127.0.0.1:11434",
        );
        add_subview(&sec_chat, &chat_base_url_field);

        add_subview(
            &sec_chat,
            &make_label(mtm, top_rect(sec_h, 0.0, 100.0, 72.0, 18.0), "API 密钥", 12.0, false, true),
        );
        let chat_key_field = make_secure_field(
            mtm,
            top_rect(sec_h, 74.0, 98.0, content_w - 190.0, 23.0),
            if chat_cfg.has_api_key {
                "已保存，留空保持不变"
            } else {
                "sk-… (本地服务/GGUF免填)"
            },
        );
        let chat_key_status_label = make_label(
            mtm,
            top_rect(sec_h, content_w - 112.0, 100.0, 50.0, 18.0),
            if chat_cfg.has_api_key { "已保存" } else { "未设置" },
            11.0,
            false,
            true,
        );
        let btn_chat_clr_key = make_button(
            mtm,
            top_rect(sec_h, content_w - 60.0, 97.0, 60.0, 24.0),
            "清除",
            ActionTag::ChatClearKey as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_chat, &chat_key_field);
        add_subview(&sec_chat, &chat_key_status_label);
        add_subview(&sec_chat, &btn_chat_clr_key);

        add_subview(
            &sec_chat,
            &make_label(mtm, top_rect(sec_h, 0.0, 132.0, 72.0, 18.0), "模型名称", 12.0, false, true),
        );
        let chat_model_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 130.0, content_w - 216.0, 23.0),
            &chat_cfg.model,
            "gpt-4o-mini / qwen2.5:7b",
        );
        let btn_chat_discover = make_button(
            mtm,
            top_rect(sec_h, content_w - 138.0, 129.0, 66.0, 25.0),
            "探测模型",
            ActionTag::ChatDiscoverModels as isize,
            dispatcher,
            action_sel,
        );
        let chat_discovered_popup = make_popup(
            mtm,
            top_rect(sec_h, content_w - 70.0, 129.0, 70.0, 25.0),
            &["选择…"],
            0,
            true,
            ActionTag::ChatModelPopupSelected as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_chat, &chat_model_field);
        add_subview(&sec_chat, &btn_chat_discover);
        add_subview(&sec_chat, &chat_discovered_popup);

        add_subview(
            &sec_chat,
            &make_label(mtm, top_rect(sec_h, 0.0, 164.0, 72.0, 18.0), "GGUF模型", 12.0, false, true),
        );
        let chat_gguf_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 162.0, content_w - 138.0, 23.0),
            &chat_cfg.gguf_model_path,
            "本地 .gguf 模型文件路径",
        );
        let btn_chat_gguf = make_button(
            mtm,
            top_rect(sec_h, content_w - 60.0, 161.0, 60.0, 25.0),
            "浏览…",
            ActionTag::ChatBrowseGguf as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_chat, &chat_gguf_field);
        add_subview(&sec_chat, &btn_chat_gguf);

        add_subview(
            &sec_chat,
            &make_label(mtm, top_rect(sec_h, 0.0, 196.0, 72.0, 18.0), "系统提示词", 12.0, false, true),
        );
        let chat_prompt_field = make_text_field(
            mtm,
            top_rect(sec_h, 74.0, 194.0, content_w - 74.0, 23.0),
            &chat_cfg.chat_prompt,
            "可选，会作为系统提示加在对话开头",
        );
        add_subview(&sec_chat, &chat_prompt_field);

        let save_chat_btn = make_button(
            mtm,
            top_rect(sec_h, 0.0, 232.0, 96.0, 28.0),
            "保存配置",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_chat, &save_chat_btn);

        // ── Section 3: 快捷键 ──
        let sc_head = make_label(
            mtm,
            top_rect(sec_h, 0.0, 6.0, 160.0, 18.0),
            "全局快捷键",
            12.0,
            true,
            true,
        );
        let btn_reset_sc = make_button(
            mtm,
            top_rect(sec_h, content_w - 92.0, 3.0, 92.0, 24.0),
            "恢复默认热键",
            ActionTag::ResetDefaultShortcuts as isize,
            dispatcher,
            action_sel,
        );
        add_subview(&sec_shortcuts, &sc_head);
        add_subview(&sec_shortcuts, &btn_reset_sc);

        add_subview(
            &sec_shortcuts,
            &make_label(mtm, top_rect(sec_h, 0.0, 38.0, 88.0, 18.0), "呼出翻译", 12.0, false, true),
        );
        let sc_toggle_field = make_text_field(
            mtm,
            top_rect(sec_h, 90.0, 36.0, content_w - 90.0, 23.0),
            &sc_cfg.toggle,
            "CmdOrCtrl+Shift+Space",
        );
        add_subview(
            &sec_shortcuts,
            &make_label(mtm, top_rect(sec_h, 0.0, 72.0, 88.0, 18.0), "截图 OCR", 12.0, false, true),
        );
        let sc_ocr_field = make_text_field(
            mtm,
            top_rect(sec_h, 90.0, 70.0, content_w - 90.0, 23.0),
            &sc_cfg.ocr,
            "CmdOrCtrl+Shift+S",
        );
        add_subview(
            &sec_shortcuts,
            &make_label(mtm, top_rect(sec_h, 0.0, 106.0, 88.0, 18.0), "呼出 AI 对话", 12.0, false, true),
        );
        let sc_ai_field = make_text_field(
            mtm,
            top_rect(sec_h, 90.0, 104.0, content_w - 90.0, 23.0),
            &sc_cfg.ai_dialog,
            "Option+Space",
        );
        add_subview(
            &sec_shortcuts,
            &make_label(mtm, top_rect(sec_h, 0.0, 140.0, 88.0, 18.0), "划词翻译", 12.0, false, true),
        );
        let sc_sel_trans_field = make_text_field(
            mtm,
            top_rect(sec_h, 90.0, 138.0, content_w - 90.0, 23.0),
            &sc_cfg.selection_translate,
            "可与呼出翻译共用同一按键",
        );
        add_subview(
            &sec_shortcuts,
            &make_label(mtm, top_rect(sec_h, 0.0, 174.0, 88.0, 18.0), "划词 AI 对话", 12.0, false, true),
        );
        let sc_sel_ai_field = make_text_field(
            mtm,
            top_rect(sec_h, 90.0, 172.0, content_w - 90.0, 23.0),
            &sc_cfg.selection_ai_dialog,
            "可与呼出 AI 对话共用同一按键",
        );
        let save_sc_btn = make_button(
            mtm,
            top_rect(sec_h, 0.0, 212.0, 104.0, 28.0),
            "保存快捷键",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );
        let sc_hint = make_label(
            mtm,
            top_rect(sec_h, 0.0, 248.0, content_w, 54.0),
            "每个快捷键都可单独设置或留空关闭。「呼出翻译」和「划词翻译」可共用同一个键：有选中文本时走划词，无选中文本则呼出窗口。",
            11.0,
            false,
            true,
        );
        add_subview(&sec_shortcuts, &sc_toggle_field);
        add_subview(&sec_shortcuts, &sc_ocr_field);
        add_subview(&sec_shortcuts, &sc_ai_field);
        add_subview(&sec_shortcuts, &sc_sel_trans_field);
        add_subview(&sec_shortcuts, &sc_sel_ai_field);
        add_subview(&sec_shortcuts, &save_sc_btn);
        add_subview(&sec_shortcuts, &sc_hint);

        // ── Section 4: 通用 ──
        add_subview(
            &sec_general,
            &make_label(mtm, top_rect(sec_h, 0.0, 6.0, 120.0, 18.0), "通用", 12.0, true, true),
        );
        add_subview(
            &sec_general,
            &make_label(mtm, top_rect(sec_h, 0.0, 38.0, 88.0, 18.0), "面板透明度", 12.5, false, false),
        );
        let opacity_val_label = make_label(
            mtm,
            top_rect(sec_h, content_w - 52.0, 38.0, 52.0, 18.0),
            &format!("{:.0}%", app_cfg.panel_opacity * 100.0),
            12.0,
            true,
            true,
        );
        let opacity_slider = make_slider(
            mtm,
            top_rect(sec_h, 0.0, 62.0, content_w, 22.0),
            PANEL_OPACITY_MIN,
            PANEL_OPACITY_MAX,
            app_cfg.panel_opacity,
            ActionTag::OpacitySliderChanged as isize,
            dispatcher,
            action_sel,
        );
        let autostart_chk = make_checkbox(
            mtm,
            top_rect(sec_h, 0.0, 98.0, 220.0, 22.0),
            "开机自动启动",
            ttree_platform_macos::autostart::is_enabled(),
            ActionTag::AutostartToggled as isize,
            dispatcher,
            action_sel,
        );
        let ver_label = make_label(
            mtm,
            top_rect(sec_h, 0.0, 136.0, content_w, 20.0),
            "版本：0.2.0 (100% Pure Rust Native AppKit)",
            12.0,
            false,
            true,
        );
        let gen_hint = make_label(
            mtm,
            top_rect(sec_h, 0.0, 166.0, content_w, 72.0),
            "调整面板透明度可让底层桌面毛玻璃透出。\n截图 OCR 需要“屏幕录制”权限；划词翻译优先使用“辅助功能”权限，未授权时会临时用复制代替。\n所有配置直接保存在本地 JSON，不再调用 macOS 钥匙串。",
            11.5,
            false,
            true,
        );
        add_subview(&sec_general, &opacity_val_label);
        add_subview(&sec_general, &opacity_slider);
        add_subview(&sec_general, &autostart_chk);
        add_subview(&sec_general, &ver_label);
        add_subview(&sec_general, &gen_hint);

        let settings_status_label = make_label(
            mtm,
            top_rect(h, content_x, h - 28.0, content_w, 20.0),
            "",
            12.0,
            true,
            false,
        );

        add_subview(&root, &sec_translate);
        add_subview(&root, &sec_chat);
        add_subview(&root, &sec_shortcuts);
        add_subview(&root, &sec_general);
        add_subview(&root, &settings_status_label);

        SettingsViewWidgets {
            root,
            top_divider,
            sidebar_divider,
            nav_translate_btn,
            nav_chat_btn,
            nav_shortcuts_btn,
            nav_general_btn,
            sec_translate,
            sec_chat,
            sec_shortcuts,
            sec_general,
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
            settings_status_label,
        }
    }

    pub fn set_status(&self, msg: &str) {
        set_field_string(&self.translate_ui.status_label, msg);
        set_field_string(&self.settings_ui.settings_status_label, msg);
    }

    pub fn switch_settings_section(&mut self, sec: SettingsSection) {
        self.active_settings_sec = sec;
        self.settings_ui
            .sec_translate
            .setHidden(sec != SettingsSection::Translate);
        self.settings_ui
            .sec_chat
            .setHidden(sec != SettingsSection::Chat);
        self.settings_ui
            .sec_shortcuts
            .setHidden(sec != SettingsSection::Shortcuts);
        self.settings_ui
            .sec_general
            .setHidden(sec != SettingsSection::General);

        let set_nav = |btn: &NSButton, label: &str, active: bool| {
            let prefix = if active { "● " } else { "   " };
            btn.setTitle(&NSString::from_str(&format!("{prefix}{label}")));
            set_flat_button_active(btn, active, true);
        };
        set_nav(
            &self.settings_ui.nav_translate_btn,
            "AI 翻译",
            sec == SettingsSection::Translate,
        );
        set_nav(
            &self.settings_ui.nav_chat_btn,
            "AI 对话",
            sec == SettingsSection::Chat,
        );
        set_nav(
            &self.settings_ui.nav_shortcuts_btn,
            "快捷键",
            sec == SettingsSection::Shortcuts,
        );
        set_nav(
            &self.settings_ui.nav_general_btn,
            "通用",
            sec == SettingsSection::General,
        );
    }

    pub fn update_window_min_max_constraints(&self) {
        let ai_enabled = self.settings.ai_translation_for_request().enabled;
        match self.active_tab {
            TabMode::Translate => {
                self.window.setMinSize(NSSize::new(
                    WINDOW_MIN_WIDTH,
                    PaneLayoutConfig::min_window_height(ai_enabled),
                ));
                self.window.setMaxSize(NSSize::new(
                    WINDOW_MAX_WIDTH,
                    PaneLayoutConfig::max_window_height(ai_enabled),
                ));
            }
            TabMode::Chat => {
                self.window
                    .setMinSize(NSSize::new(WINDOW_MIN_WIDTH, 320.0));
                self.window
                    .setMaxSize(NSSize::new(WINDOW_MAX_WIDTH, WINDOW_MAX_HEIGHT));
            }
            TabMode::Settings => {
                self.window
                    .setMinSize(NSSize::new(WINDOW_MIN_WIDTH, 430.0));
                self.window
                    .setMaxSize(NSSize::new(WINDOW_MAX_WIDTH, WINDOW_MAX_HEIGHT));
            }
        }
    }

    /// 将窗口尺寸严格同步至记忆的 `pane_layout`（顶边在屏幕上的位置保持固定不动）。
    pub fn sync_window_to_pane_layout(&mut self) {
        self.pane_layout = self.pane_layout.clamped();
        let ai_enabled = self.settings.ai_translation_for_request().enabled;
        self.update_window_min_max_constraints();

        let target_w = self.pane_layout.width;
        let target_h = match self.active_tab {
            TabMode::Translate => self.pane_layout.translate_window_height(ai_enabled),
            TabMode::Chat => self
                .pane_layout
                .translate_window_height(ai_enabled)
                .max(360.0),
            TabMode::Settings => self
                .pane_layout
                .translate_window_height(ai_enabled)
                .max(450.0),
        };

        let frame = self.window.frame();
        let top_y = frame.origin.y + frame.size.height;
        if (frame.size.width - target_w).abs() >= 0.5 || (frame.size.height - target_h).abs() >= 0.5
        {
            let new_frame = NSRect::new(
                NSPoint::new(frame.origin.x, top_y - target_h),
                NSSize::new(target_w, target_h),
            );
            self.is_programmatic_resize = true;
            self.window.setFrame_display(new_frame, true);
            self.is_programmatic_resize = false;
        }
        self.last_known_size = (target_w, target_h);
        self.relayout_all(target_w, target_h);
    }

    pub fn switch_tab(&mut self, tab: TabMode) {
        if self.active_tab == TabMode::Settings && tab != TabMode::Settings {
            self.set_pinned(self.pin_before_settings);
        } else if self.active_tab != TabMode::Settings && tab == TabMode::Settings {
            self.pin_before_settings = self.is_pinned;
            self.set_pinned(true);
        }

        if tab != TabMode::Settings {
            self.last_main_mode = tab;
        }
        self.active_tab = tab;

        self.translate_ui.root.setHidden(tab != TabMode::Translate);
        self.chat_ui.root.setHidden(tab != TabMode::Chat);
        self.settings_ui.root.setHidden(tab != TabMode::Settings);

        let in_settings = tab == TabMode::Settings;
        self.segmented_bg.setHidden(in_settings);
        self.segmented_pill.setHidden(in_settings);
        self.tab_translate_btn.setHidden(in_settings);
        self.tab_chat_btn.setHidden(in_settings);
        self.settings_title_label.setHidden(!in_settings);
        self.gear_btn.setHidden(in_settings);
        self.done_settings_btn.setHidden(!in_settings);
        self.new_chat_top_btn.setHidden(tab != TabMode::Chat);
        self.history_top_btn.setHidden(tab != TabMode::Chat);

        if !in_settings {
            set_flat_button_active(&self.tab_translate_btn, tab == TabMode::Translate, false);
            set_flat_button_active(&self.tab_chat_btn, tab == TabMode::Chat, false);
        }

        self.sync_window_to_pane_layout();

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
        let sym = if pinned { "pin.fill" } else { "pin" };
        set_button_symbol(&self.pin_btn, sym, "📌", pinned);
    }

    pub fn show_and_focus_window(&mut self) {
        let was_visible = self.window.isVisible();
        self.sync_window_to_pane_layout();

        let app = NSApplication::sharedApplication(self.mtm);
        if !was_visible {
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
        if self.pending_resize_save.take().is_some() {
            self.settings.save_pane_layout(self.pane_layout);
        }
        self.window.orderOut(None);
    }

    pub fn refresh_ai_header_badge(&self) {
        let cfg = self.settings.ai_translation_for_request();
        let label = if !cfg.enabled {
            "AI (未开启)".to_string()
        } else {
            match cfg.backend {
                AiBackendKind::OpenAiCompatible => format!("AI  ·  {}", cfg.model),
                AiBackendKind::Ollama => format!("AI  ·  OLLAMA ({})", cfg.model),
                AiBackendKind::LocalServer => format!("AI  ·  LOCAL ({})", cfg.model),
                AiBackendKind::CandleGguf => {
                    let name = std::path::Path::new(&cfg.gguf_model_path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("GGUF");
                    format!("AI  ·  {name}")
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
        self.translate_ui
            .input_placeholder
            .setHidden(!raw_text.is_empty());

        if trimmed.is_empty() {
            self.translate_cancel.cancel_current();
            self.web_has_content = false;
            self.ai_has_content = false;
            self.translate_ui.web_copy_btn.setHidden(true);
            self.translate_ui.ai_copy_btn.setHidden(true);
            set_text_view_styled(&self.translate_ui.web_tv, WEB_PLACEHOLDER, true);
            set_text_view_styled(&self.translate_ui.ai_tv, AI_PLACEHOLDER, true);
            self.last_translated_source.clear();
            self.set_status("");
            return;
        }

        let cancel_token = self.translate_cancel.next();
        self.translate_gen = self.translate_gen.wrapping_add(1);
        let gen = self.translate_gen;
        self.last_translated_source = trimmed.clone();
        self.web_has_content = false;
        self.ai_has_content = false;
        self.translate_ui.web_copy_btn.setHidden(true);
        self.translate_ui.ai_copy_btn.setHidden(true);

        let src_ui = self.selected_source_code();
        let tgt_ui = self.selected_target_code();
        let (resolved_src, resolved_tgt) = resolve_language_pair(&trimmed, src_ui, tgt_ui);

        set_text_view_styled(&self.translate_ui.web_tv, WEB_LOADING_TEXT, true);
        let ai_cfg = self.settings.ai_translation_for_request();
        self.refresh_ai_header_badge();
        if ai_cfg.enabled {
            set_text_view_styled(&self.translate_ui.ai_tv, AI_LOADING_TEXT, true);
        } else {
            set_text_view_styled(&self.translate_ui.ai_tv, AI_PLACEHOLDER, true);
        }
        self.set_status("");

        // 1. 并发启动谷歌/微软负载均衡竞速翻译
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

        // 2. 并发启动 AI 流式翻译
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
            ActionTag::CloseSettings => {
                self.switch_tab(self.last_main_mode);
            }
            ActionTag::SettingsSecTranslate => {
                self.switch_settings_section(SettingsSection::Translate);
            }
            ActionTag::SettingsSecChat => {
                self.switch_settings_section(SettingsSection::Chat);
            }
            ActionTag::SettingsSecShortcuts => {
                self.switch_settings_section(SettingsSection::Shortcuts);
            }
            ActionTag::SettingsSecGeneral => {
                self.switch_settings_section(SettingsSection::General);
            }
            ActionTag::TriggerOcr => self.trigger_ocr(),
            ActionTag::TogglePin => self.set_pinned(!self.is_pinned),
            ActionTag::TrayQuit => {
                let app = NSApplication::sharedApplication(self.mtm);
                app.terminate(None);
            }
            ActionTag::HideOrEscape => {
                if self.active_tab == TabMode::Settings {
                    self.switch_tab(self.last_main_mode);
                } else if self.active_tab == TabMode::Chat && self.show_chat_history_bar {
                    self.toggle_chat_history_bar();
                } else {
                    self.hide_window();
                }
            }
            ActionTag::TranslateNow => self.start_translation(),
            ActionTag::ClearTranslate => {
                self.translate_cancel.cancel_current();
                set_text_view_string(&self.translate_ui.input_tv, "");
                self.translate_ui.input_placeholder.setHidden(false);
                self.web_has_content = false;
                self.ai_has_content = false;
                self.translate_ui.web_copy_btn.setHidden(true);
                self.translate_ui.ai_copy_btn.setHidden(true);
                set_text_view_styled(&self.translate_ui.web_tv, WEB_PLACEHOLDER, true);
                set_text_view_styled(&self.translate_ui.ai_tv, AI_PLACEHOLDER, true);
                self.last_observed_source.clear();
                self.last_translated_source.clear();
                self.pending_debounce_since = None;
                self.set_status("");
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
                if self.web_has_content {
                    let text = get_text_view_string(&self.translate_ui.web_tv);
                    if copy_to_pasteboard(&text) {
                        self.translate_ui
                            .web_copy_btn
                            .setTitle(&NSString::from_str("已复制"));
                    }
                }
            }
            ActionTag::CopyAiResult => {
                if self.ai_has_content {
                    let text = get_text_view_string(&self.translate_ui.ai_tv);
                    if copy_to_pasteboard(&text) {
                        self.translate_ui
                            .ai_copy_btn
                            .setTitle(&NSString::from_str("已复制"));
                    }
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
            }
            ActionTag::StopChatStream => {
                self.chat_cancel.cancel_current();
                self.chat_streaming = false;
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
                    let _ = copy_to_pasteboard(&last.content);
                }
            }
            ActionTag::ToggleHistoryBar => {
                self.toggle_chat_history_bar();
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
                self.set_status("✓ 已同步翻译模型配置，请点击保存");
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
                self.set_status("已填入默认快捷键，请点击「保存快捷键」");
            }
            ActionTag::OpacitySliderChanged => {
                let val = AppearanceConfig::clamp_opacity(
                    self.settings_ui.opacity_slider.doubleValue(),
                );
                self.window.setAlphaValue(val);
                let _ = self.settings.save_appearance(AppearanceConfig {
                    panel_opacity: val,
                });
                set_field_string(
                    &self.settings_ui.opacity_val_label,
                    &format!("{:.0}%", val * 100.0),
                );
            }
            ActionTag::AutostartToggled => {
                let want = self.settings_ui.autostart_chk.state() == NSControlStateValueOn;
                let _ = ttree_platform_macos::autostart::set_enabled(want);
            }
            ActionTag::SaveSettings => self.save_all_settings(),
        }
    }

    fn toggle_chat_history_bar(&mut self) {
        self.show_chat_history_bar = !self.show_chat_history_bar;
        self.chat_ui
            .history_bar
            .setHidden(!self.show_chat_history_bar);
        set_button_symbol(
            &self.history_top_btn,
            "clock.arrow.circlepath",
            "⏱",
            self.show_chat_history_bar,
        );
        self.relayout_chat_view(
            self.last_known_size.0,
            (self.last_known_size.1 - TOP_BAR_HEIGHT).max(200.0),
        );
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
        self.set_status("正在探测本机模型…");
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
                self.set_status(&format!("⚠️ 请选择 .{} 文件", allowed_exts.join("/")));
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
            self.set_status(&format!("❌ {e}"));
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
            self.set_status(&format!("❌ {e}"));
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
                self.set_status(&format!("❌ {e}"));
                return;
            }
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
        self.set_status("✓ 已保存");
    }

    pub fn send_chat_from_input(&mut self) {
        let raw = get_text_view_string(&self.chat_ui.input_tv);
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        set_text_view_string(&self.chat_ui.input_tv, "");
        self.chat_ui.input_placeholder.setHidden(false);
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
        if self.current_messages.is_empty() && !self.chat_streaming {
            set_text_view_styled(&self.chat_ui.transcript_tv, "和 AI 自由对话。", true);
            return;
        }
        let mut out = String::new();
        for msg in &self.current_messages {
            let prefix = if msg.role == "user" { "你" } else { "AI" };
            out.push_str(prefix);
            out.push('\n');
            out.push_str(&msg.content);
            out.push_str("\n\n");
        }
        if self.chat_streaming {
            out.push_str("AI\n");
            out.push_str(&self.streaming_assistant_buffer);
            out.push_str(" ▍");
        }
        set_text_view_styled(&self.chat_ui.transcript_tv, &out, false);
        let len = NSString::from_str(&out).length();
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
                        self.web_has_content = !translated.trim().is_empty();
                        self.translate_ui
                            .web_copy_btn
                            .setTitle(&NSString::from_str("复制"));
                        self.translate_ui
                            .web_copy_btn
                            .setHidden(!self.web_has_content);
                        set_text_view_styled(&self.translate_ui.web_tv, &translated, false);
                    }
                    Err(err) => {
                        self.web_has_content = false;
                        self.translate_ui.web_copy_btn.setHidden(true);
                        set_text_view_styled(&self.translate_ui.web_tv, &err, false);
                    }
                }
            }
            UiEvent::AiTranslateDelta { gen, delta } => {
                if gen != self.translate_gen {
                    return;
                }
                if !self.ai_has_content {
                    self.ai_has_content = true;
                    self.translate_ui
                        .ai_copy_btn
                        .setTitle(&NSString::from_str("复制"));
                    self.translate_ui.ai_copy_btn.setHidden(false);
                    set_text_view_styled(&self.translate_ui.ai_tv, &delta, false);
                } else {
                    append_text_view_string(&self.translate_ui.ai_tv, &delta, true);
                }
            }
            UiEvent::AiTranslateFinished { gen, error } => {
                if gen != self.translate_gen {
                    return;
                }
                if let Some(err) = error {
                    self.ai_has_content = false;
                    self.translate_ui.ai_copy_btn.setHidden(true);
                    set_text_view_styled(&self.translate_ui.ai_tv, &err, false);
                }
            }
            UiEvent::OcrCompleted { outcome } => match outcome {
                Ok(OcrOutcome::Text(text)) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_string(&self.translate_ui.input_tv, &text);
                    self.translate_ui.input_placeholder.setHidden(true);
                    self.last_observed_source = text;
                    self.start_translation();
                }
                Ok(OcrOutcome::Cancelled) | Ok(OcrOutcome::Busy) => {}
                Ok(OcrOutcome::EmptyText) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_styled(
                        &self.translate_ui.web_tv,
                        "所选截图区域未识别到文字，请重新框选。",
                        true,
                    );
                }
                Ok(OcrOutcome::PermissionDenied(msg)) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_styled(&self.translate_ui.web_tv, &msg, false);
                }
                Err(err) => {
                    self.switch_tab(TabMode::Translate);
                    self.show_and_focus_window();
                    set_text_view_styled(&self.translate_ui.web_tv, &err, false);
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
                    self.translate_ui.input_placeholder.setHidden(true);
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
                    self.set_status(&format!("✓ 发现 {} 个本地模型", models.len()));
                }
                Err(err) => {
                    self.set_status(&format!("⚠️ {err}"));
                }
            },
        }
    }

    pub fn poll_tick(&mut self) {
        if self.window.isVisible() {
            match self.active_tab {
                TabMode::Translate => {
                    let current_src = get_text_view_string(&self.translate_ui.input_tv);
                    self.translate_ui
                        .input_placeholder
                        .setHidden(!current_src.is_empty());
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
                TabMode::Chat => {
                    let chat_in = get_text_view_string(&self.chat_ui.input_tv);
                    self.chat_ui.input_placeholder.setHidden(!chat_in.is_empty());
                }
                TabMode::Settings => {}
            }
        }

        if let Some(since) = self.pending_resize_save {
            if since.elapsed() >= RESIZE_SAVE_DEBOUNCE {
                self.pending_resize_save = None;
                self.settings.save_pane_layout(self.pane_layout);
            }
        }
    }

    pub fn on_window_did_resign_key(&mut self) {
        if !self.is_pinned && self.active_tab != TabMode::Settings && !self.is_modal_open {
            self.hide_window();
        }
    }

    pub fn on_splitter_drag(&mut self, splitter_id: u8, delta_down: f64) {
        match splitter_id {
            SPLITTER_SOURCE => {
                self.pane_layout.adjust_source_height(delta_down);
                self.sync_window_to_pane_layout();
            }
            SPLITTER_LANG => {
                self.pane_layout.adjust_lang_height(delta_down);
                self.sync_window_to_pane_layout();
            }
            SPLITTER_WEB => {
                self.pane_layout.adjust_web_height(delta_down);
                self.sync_window_to_pane_layout();
            }
            SPLITTER_CHAT => {
                self.pane_layout.adjust_chat_input_height(-delta_down);
                let (w, h) = self.last_known_size;
                self.relayout_chat_view(w, (h - TOP_BAR_HEIGHT).max(200.0));
            }
            _ => return,
        }
        self.pending_resize_save = Some(Instant::now());
    }

    pub fn on_splitter_drag_end(&mut self) {
        self.pending_resize_save = None;
        self.settings.save_pane_layout(self.pane_layout);
    }

    pub fn on_window_did_resize(&mut self) {
        let Some(content_view) = self.window.contentView() else {
            return;
        };
        let bounds = content_view.bounds();
        let w = bounds
            .size
            .width
            .clamp(WINDOW_MIN_WIDTH, WINDOW_MAX_WIDTH);
        let h = bounds
            .size
            .height
            .clamp(WINDOW_MIN_HEIGHT, WINDOW_MAX_HEIGHT);
        if (w - self.last_known_size.0).abs() < 0.5 && (h - self.last_known_size.1).abs() < 0.5 {
            return;
        }
        self.last_known_size = (w, h);

        if !self.is_programmatic_resize {
            let ai_enabled = self.settings.ai_translation_for_request().enabled;
            match self.active_tab {
                TabMode::Translate => {
                    self.pane_layout.apply_window_resize(w, h, ai_enabled);
                }
                TabMode::Chat | TabMode::Settings => {
                    self.pane_layout.width = w;
                }
            }
            self.pending_resize_save = Some(Instant::now());
        }

        self.relayout_all(w, h);
    }

    pub fn relayout_all(&self, w: f64, h: f64) {
        self.effect_view.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(w, h),
        ));

        // 顶栏重排
        self.segmented_bg
            .setFrame(top_rect(h, 12.0, 5.0, 128.0, 28.0));
        let pill_x = if self.active_tab == TabMode::Chat {
            78.0
        } else {
            15.0
        };
        self.segmented_pill
            .setFrame(top_rect(h, pill_x, 7.0, 59.0, 24.0));
        self.tab_translate_btn
            .setFrame(top_rect(h, 15.0, 7.0, 59.0, 24.0));
        self.tab_chat_btn
            .setFrame(top_rect(h, 78.0, 7.0, 59.0, 24.0));
        self.settings_title_label
            .setFrame(top_rect(h, 16.0, 8.0, 120.0, 22.0));

        let pin_x = match self.active_tab {
            TabMode::Translate => w - 76.0,
            TabMode::Chat => w - 140.0,
            TabMode::Settings => w - 92.0,
        };
        self.pin_btn.setFrame(top_rect(h, pin_x, 5.0, 28.0, 28.0));
        self.new_chat_top_btn
            .setFrame(top_rect(h, w - 108.0, 5.0, 28.0, 28.0));
        self.history_top_btn
            .setFrame(top_rect(h, w - 76.0, 5.0, 28.0, 28.0));
        self.gear_btn
            .setFrame(top_rect(h, w - 42.0, 5.0, 28.0, 28.0));
        self.done_settings_btn
            .setFrame(top_rect(h, w - 56.0, 6.0, 44.0, 26.0));

        let body_h = (h - TOP_BAR_HEIGHT).max(174.0);
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, body_h));

        // 翻译上下四层分栏重排（严格遵循每栏高度与最小/最大限高）
        self.translate_ui.root.setFrame(body_frame);
        let ai_enabled = self.settings.ai_translation_for_request().enabled;
        let layout = self.pane_layout.clamped();
        let source_h = layout.source_height;
        let lang_h = layout.lang_height;
        let results_top = source_h + lang_h;
        let web_h = if ai_enabled {
            layout.web_height
        } else {
            (body_h - results_top).max(layout.web_height)
        };
        let ai_top = results_top + web_h;
        let ai_h = (body_h - ai_top).max(layout.ai_height);

        self.translate_ui
            .source_surface
            .setFrame(top_rect(body_h, 0.0, 0.0, w, source_h));
        self.translate_ui.input_scroll.setFrame(top_rect(
            body_h,
            2.0,
            2.0,
            w - 4.0,
            (source_h - 6.0).max(40.0),
        ));
        self.translate_ui
            .input_placeholder
            .setFrame(top_rect(body_h, 16.0, 12.0, w - 32.0, 20.0));
        set_split_bar_frame(
            &self.translate_ui.source_split_bar,
            top_rect(
                body_h,
                0.0,
                source_h - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
        );

        let popup_top = source_h + ((lang_h - 24.0) * 0.5).max(2.0);
        self.translate_ui
            .lang_surface
            .setFrame(top_rect(body_h, 0.0, source_h, w, lang_h));
        self.translate_ui
            .source_popup
            .setFrame(top_rect(body_h, 12.0, popup_top, 114.0, 24.0));
        self.translate_ui
            .swap_btn
            .setFrame(top_rect(body_h, 128.0, popup_top, 24.0, 24.0));
        self.translate_ui
            .target_popup
            .setFrame(top_rect(body_h, 154.0, popup_top, 152.0, 24.0));
        self.translate_ui.status_label.setFrame(top_rect(
            body_h,
            310.0,
            popup_top + 3.0,
            (w - 322.0).max(60.0),
            18.0,
        ));
        set_split_bar_frame(
            &self.translate_ui.lang_split_bar,
            top_rect(
                body_h,
                0.0,
                results_top - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
        );

        self.translate_ui
            .web_header_label
            .setFrame(top_rect(body_h, 16.0, results_top + 7.0, 240.0, 15.0));
        self.translate_ui
            .web_copy_btn
            .setFrame(top_rect(body_h, w - 58.0, results_top + 4.0, 46.0, 20.0));
        self.translate_ui.web_scroll.setFrame(top_rect(
            body_h,
            2.0,
            results_top + 22.0,
            w - 4.0,
            (web_h - 26.0).max(40.0),
        ));
        set_split_bar_frame(
            &self.translate_ui.web_split_bar,
            top_rect(
                body_h,
                0.0,
                results_top + web_h - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
        );

        self.translate_ui
            .web_split_bar
            .handle
            .setHidden(!ai_enabled);
        self.translate_ui.ai_header_label.setHidden(!ai_enabled);
        self.translate_ui.ai_scroll.setHidden(!ai_enabled);
        if !ai_enabled {
            self.translate_ui.ai_copy_btn.setHidden(true);
        } else if self.ai_has_content {
            self.translate_ui.ai_copy_btn.setHidden(false);
        }

        self.translate_ui
            .ai_header_label
            .setFrame(top_rect(body_h, 16.0, ai_top + 7.0, 280.0, 15.0));
        self.translate_ui
            .ai_copy_btn
            .setFrame(top_rect(body_h, w - 58.0, ai_top + 4.0, 46.0, 20.0));
        self.translate_ui.ai_scroll.setFrame(top_rect(
            body_h,
            2.0,
            ai_top + 22.0,
            w - 4.0,
            (ai_h - 26.0).max(40.0),
        ));

        // 对话视图重排
        self.relayout_chat_view(w, body_h);

        // 设置视图重排
        self.settings_ui.root.setFrame(body_frame);
        self.settings_ui
            .top_divider
            .setFrame(top_rect(body_h, 12.0, 0.0, w - 24.0, 1.0));
        let sidebar_w = 114.0;
        self.settings_ui
            .sidebar_divider
            .setFrame(top_rect(body_h, sidebar_w, 1.0, 1.0, body_h - 1.0));
        self.settings_ui
            .nav_translate_btn
            .setFrame(top_rect(body_h, 10.0, 12.0, sidebar_w - 20.0, 30.0));
        self.settings_ui
            .nav_chat_btn
            .setFrame(top_rect(body_h, 10.0, 46.0, sidebar_w - 20.0, 30.0));
        self.settings_ui
            .nav_shortcuts_btn
            .setFrame(top_rect(body_h, 10.0, 80.0, sidebar_w - 20.0, 30.0));
        self.settings_ui
            .nav_general_btn
            .setFrame(top_rect(body_h, 10.0, 114.0, sidebar_w - 20.0, 30.0));
    }

    fn relayout_chat_view(&self, w: f64, body_h: f64) {
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, body_h));
        self.chat_ui.root.setFrame(body_frame);
        let hist_offset = if self.show_chat_history_bar { 34.0 } else { 0.0 };
        self.chat_ui
            .history_bar
            .setFrame(top_rect(body_h, 0.0, 0.0, w, 34.0));
        self.chat_ui
            .conv_popup
            .setFrame(top_rect(34.0, 12.0, 4.0, w - 104.0, 24.0));
        self.chat_ui
            .clear_hist_btn
            .setFrame(top_rect(34.0, w - 84.0, 4.0, 72.0, 24.0));
        self.chat_ui
            .history_bottom_line
            .setFrame(top_rect(34.0, 0.0, 33.0, w, 1.0));

        let composer_h = self
            .pane_layout
            .clamped()
            .chat_input_height
            .min((body_h - hist_offset - 100.0).max(68.0));
        let transcript_h = (body_h - hist_offset - composer_h).max(100.0);
        self.chat_ui.transcript_scroll.setFrame(top_rect(
            body_h,
            4.0,
            hist_offset,
            w - 8.0,
            transcript_h,
        ));
        let composer_top = hist_offset + transcript_h;
        set_split_bar_frame(
            &self.chat_ui.composer_split_bar,
            top_rect(
                body_h,
                0.0,
                composer_top - SPLITTER_HIT_HEIGHT * 0.5,
                w,
                SPLITTER_HIT_HEIGHT,
            ),
        );
        self.chat_ui.composer_surface.setFrame(top_rect(
            body_h,
            0.0,
            composer_top + 1.0,
            w,
            composer_h - 1.0,
        ));
        self.chat_ui.input_scroll.setFrame(top_rect(
            body_h,
            4.0,
            composer_top + 6.0,
            w - 84.0,
            (composer_h - 12.0).max(40.0),
        ));
        self.chat_ui
            .input_placeholder
            .setFrame(top_rect(body_h, 18.0, composer_top + 15.0, 200.0, 20.0));
        self.chat_ui.send_btn.setFrame(top_rect(
            body_h,
            w - 74.0,
            composer_top + (composer_h - 28.0) * 0.5,
            62.0,
            28.0,
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
            ActionTag::CloseSettings,
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
            ActionTag::ToggleHistoryBar,
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
            ActionTag::AutostartToggled,
            ActionTag::SettingsSecTranslate,
            ActionTag::SettingsSecChat,
            ActionTag::SettingsSecShortcuts,
            ActionTag::SettingsSecGeneral,
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
