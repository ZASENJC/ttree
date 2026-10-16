//! `TTREE` 核心控制器：复刻原版极简上下分栏排版（右上角功能区 + 上方翻译输入框 + 语言选择条 + Google/Bing 负载均衡译文框 + AI 译文框）。

#![cfg(target_os = "macos")]
#![allow(clippy::too_many_arguments)]

use crate::shortcut_recorder::ShortcutField;

use std::sync::Arc;
use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSBox, NSBoxType, NSButton, NSControlStateValueOn, NSFloatingWindowLevel,
    NSModalResponseOK, NSNormalWindowLevel, NSOpenPanel, NSPopUpButton, NSScreen, NSScrollView,
    NSSecureTextField, NSSlider, NSTextField, NSTextView, NSView, NSVisualEffectView, NSWindow,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::UnboundedSender;

use ttree_core::{
    recent_chat_context, resolve_language_pair, AiBackendKind, AiTranslationConfig,
    AppearanceConfig, CancelSlot, ChatAiConfig, ChatMessage, Conversation, HyMtQuant,
    PaneLayoutConfig, ShortcutConfig, ADAPTIVE_TARGET, PANEL_OPACITY_MAX, PANEL_OPACITY_MIN,
    SOURCE_LANGUAGES, TARGET_LANGUAGES, TOP_BAR_HEIGHT, WINDOW_MAX_HEIGHT, WINDOW_MAX_WIDTH,
    WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH,
};
use ttree_ocr::OcrOutcome;
use ttree_platform::hotkey::{
    parse_shortcut_config, register_shortcuts, ParsedShortcuts, ShortcutAction,
};
use ttree_storage::{HistoryStore, SettingsStore};

use crate::ui_helpers::{
    add_subview, append_text_view_string, apply_text_view_line_spacing, clean_markdown_for_bubble,
    clear_subviews, copy_to_pasteboard, current_ink_tone, get_field_string, get_text_view_string,
    is_dark_appearance, make_button, make_checkbox, make_divider, make_flat_button,
    make_flipped_view, make_label, make_multiline_input, make_popup, make_scrollable_text_view,
    make_secure_field, make_slider, make_split_bar, make_surface_box, make_symbol_button,
    make_text_field, make_wrapping_label, make_wrapping_message_label, mark_view_tree_needs_display,
    pale_ink, sample_window_backdrop_luminance, set_button_symbol, set_field_string,
    set_flat_button_active, set_ink_tone, set_label_alignment_right, set_multiline_input_frame,
    set_multiline_input_hidden, set_split_bar_frame, set_text_view_string, set_text_view_styled,
    set_wrapping_label_frame, step_ink_tone, target_ink_tone_for_luminance, top_rect, FlippedView,
    MultilineInputWidget, SplitBarWidget, SPLITTER_HIT_HEIGHT,
};

const AUTO_TRANSLATE_DEBOUNCE: Duration = Duration::from_millis(650);
const RESIZE_SAVE_DEBOUNCE: Duration = Duration::from_millis(350);
const BACKDROP_SAMPLE_INTERVAL: Duration = Duration::from_millis(120);

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
    OcrStripNewlinesToggled = 53,

    SettingsSecTranslate = 60,
    SettingsSecChat = 61,
    SettingsSecShortcuts = 62,
    SettingsSecGeneral = 63,
    UpdateApp = 64,

    HyMtQuantChanged = 70,
    HyMtCheckCompatibility = 71,
    HyMtDownloadAndRun = 72,
    HyMtStopServer = 73,
    HyMtDeleteModel = 74,
    ChatHyMtQuantChanged = 75,
    ChatHyMtDownloadAndRun = 76,
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
            53 => Some(Self::OcrStripNewlinesToggled),
            60 => Some(Self::SettingsSecTranslate),
            61 => Some(Self::SettingsSecChat),
            62 => Some(Self::SettingsSecShortcuts),
            63 => Some(Self::SettingsSecGeneral),
            64 => Some(Self::UpdateApp),
            70 => Some(Self::HyMtQuantChanged),
            71 => Some(Self::HyMtCheckCompatibility),
            72 => Some(Self::HyMtDownloadAndRun),
            73 => Some(Self::HyMtStopServer),
            74 => Some(Self::HyMtDeleteModel),
            75 => Some(Self::ChatHyMtQuantChanged),
            76 => Some(Self::ChatHyMtDownloadAndRun),
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
    HyMtProgress {
        progress: ttree_ai::hy_mt_manager::HyMtDownloadProgress,
    },
    HyMtActionFinished {
        quant: HyMtQuant,
        result: Result<String, String>,
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
    pub history_surface: Retained<NSBox>,
    pub history_label: Retained<NSTextField>,
    pub conv_popup: Retained<NSPopUpButton>,
    pub new_conv_btn: Retained<NSButton>,
    pub clear_hist_btn: Retained<NSButton>,
    pub history_bottom_line: Retained<NSBox>,
    pub transcript_scroll: Retained<NSScrollView>,
    pub transcript_doc: Retained<FlippedView>,
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

    // Section 1: AI 翻译
    pub t_head: Retained<NSTextField>,
    pub ai_enabled_chk: Retained<NSButton>,
    pub t_lbl_backend: Retained<NSTextField>,
    pub ai_backend_popup: Retained<NSPopUpButton>,
    pub btn_ai_def_url: Retained<NSButton>,
    // BuiltInHyMt 专属控件（全精度选择 + 一键检测 + 一键下载运行 + 停止 + 删除 + 状态卡片）
    pub t_lbl_hymt_quant: Retained<NSTextField>,
    pub ai_hymt_quant_popup: Retained<NSPopUpButton>,
    pub t_lbl_hymt_action: Retained<NSTextField>,
    pub btn_hymt_check: Retained<NSButton>,
    pub btn_hymt_download_run: Retained<NSButton>,
    pub btn_hymt_stop: Retained<NSButton>,
    pub btn_hymt_delete: Retained<NSButton>,
    pub t_lbl_hymt_status: Retained<NSTextField>,
    pub hymt_info_surface: Retained<NSBox>,
    pub hymt_info_label: Retained<NSTextField>,

    pub t_lbl_url: Retained<NSTextField>,
    pub ai_base_url_field: Retained<NSTextField>,
    pub t_lbl_key: Retained<NSTextField>,
    pub ai_key_field: Retained<NSSecureTextField>,
    pub ai_key_status_label: Retained<NSTextField>,
    pub btn_ai_clr_key: Retained<NSButton>,
    pub t_lbl_model: Retained<NSTextField>,
    pub ai_model_field: Retained<NSTextField>,
    pub btn_ai_discover: Retained<NSButton>,
    pub ai_discovered_popup: Retained<NSPopUpButton>,
    pub t_lbl_gguf: Retained<NSTextField>,
    pub ai_gguf_field: Retained<NSTextField>,
    pub btn_ai_gguf: Retained<NSButton>,
    pub t_lbl_tok: Retained<NSTextField>,
    pub ai_tokenizer_field: Retained<NSTextField>,
    pub btn_ai_tok: Retained<NSButton>,
    pub t_lbl_prompt: Retained<NSTextField>,
    pub ai_prompt_input: MultilineInputWidget,
    pub save_ai_btn: Retained<NSButton>,

    // Section 2: AI 对话
    pub c_head: Retained<NSTextField>,
    pub btn_sync_from_trans: Retained<NSButton>,
    pub c_lbl_backend: Retained<NSTextField>,
    pub chat_backend_popup: Retained<NSPopUpButton>,
    pub btn_chat_def_url: Retained<NSButton>,
    pub c_lbl_hymt_quant: Retained<NSTextField>,
    pub chat_hymt_quant_popup: Retained<NSPopUpButton>,
    pub btn_chat_hymt_run: Retained<NSButton>,
    pub c_lbl_url: Retained<NSTextField>,
    pub chat_base_url_field: Retained<NSTextField>,
    pub c_lbl_key: Retained<NSTextField>,
    pub chat_key_field: Retained<NSSecureTextField>,
    pub chat_key_status_label: Retained<NSTextField>,
    pub btn_chat_clr_key: Retained<NSButton>,
    pub c_lbl_model: Retained<NSTextField>,
    pub chat_model_field: Retained<NSTextField>,
    pub btn_chat_discover: Retained<NSButton>,
    pub chat_discovered_popup: Retained<NSPopUpButton>,
    pub c_lbl_gguf: Retained<NSTextField>,
    pub chat_gguf_field: Retained<NSTextField>,
    pub btn_chat_gguf: Retained<NSButton>,
    pub c_lbl_prompt: Retained<NSTextField>,
    pub chat_prompt_input: MultilineInputWidget,
    pub save_chat_btn: Retained<NSButton>,

    // Section 3: 快捷键
    pub sc_head: Retained<NSTextField>,
    pub btn_reset_sc: Retained<NSButton>,
    pub sc_lbl_toggle: Retained<NSTextField>,
    pub sc_toggle_field: Retained<ShortcutField>,
    pub sc_lbl_ocr: Retained<NSTextField>,
    pub sc_ocr_field: Retained<ShortcutField>,
    pub sc_lbl_ai: Retained<NSTextField>,
    pub sc_ai_field: Retained<ShortcutField>,
    pub sc_lbl_sel_trans: Retained<NSTextField>,
    pub sc_sel_trans_field: Retained<ShortcutField>,
    pub sc_lbl_sel_ai: Retained<NSTextField>,
    pub sc_sel_ai_field: Retained<ShortcutField>,
    pub save_sc_btn: Retained<NSButton>,
    pub sc_hint: Retained<NSTextField>,

    // Section 4: 通用
    pub gen_head: Retained<NSTextField>,
    pub gen_lbl_opacity: Retained<NSTextField>,
    pub opacity_slider: Retained<NSSlider>,
    pub opacity_val_label: Retained<NSTextField>,
    pub autostart_chk: Retained<NSButton>,
    pub ocr_strip_newlines_chk: Retained<NSButton>,
    pub update_button: Retained<NSButton>,
    pub update_status: Retained<NSTextField>,
    pub ver_label: Retained<NSTextField>,
    pub gen_hint: Retained<NSTextField>,

    pub settings_status_label: Retained<NSTextField>,
}

pub struct AppController {
    updater: ttree_updater::Updater,
    update_view: ttree_updater::View,
    pub mtm: MainThreadMarker,
    pub rt: TokioHandle,
    pub ui_tx: UnboundedSender<UiEvent>,
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,

    pub hotkey_mgr: GlobalHotKeyManager,
    pub parsed_shortcuts: ParsedShortcuts,
    pub registered_hotkeys: Vec<HotKey>,
    pub recording_shortcut: bool,

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

    pub hymt_cancel: CancelSlot,
    pub hymt_downloading: bool,
    pub last_hymt_report: Option<ttree_ai::hy_mt_manager::HyMtCompatibilityReport>,

    pub pending_clear_ai_key: bool,
    pub pending_clear_chat_key: bool,

    pub pane_layout: PaneLayoutConfig,
    pub is_programmatic_resize: bool,
    pub last_known_size: (f64, f64),
    pub pending_resize_save: Option<Instant>,
    pub ink_tone: f64,
    pub target_ink_tone: f64,
    pub last_backdrop_sample: Option<Instant>,
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
        let app_appearance = NSApplication::sharedApplication(mtm).effectiveAppearance();
        let initial_tone = if is_dark_appearance(&app_appearance) {
            1.0
        } else {
            0.0
        };
        set_ink_tone(initial_tone);

        // ── 顶部栏：左上角 [翻译 | AI] 分段切换器，右上角图标功能区（全淡色，距顶边与左右边距留出舒适呼吸感） ──
        let segmented_bg =
            make_surface_box(mtm, top_rect(win_h, 16.0, 10.0, 128.0, 28.0), 8.0, 0.35);
        let segmented_pill =
            make_surface_box(mtm, top_rect(win_h, 19.0, 12.0, 59.0, 24.0), 6.0, 0.70);
        let tab_translate_btn = make_flat_button(
            mtm,
            top_rect(win_h, 19.0, 12.0, 59.0, 24.0),
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
            top_rect(win_h, 82.0, 12.0, 59.0, 24.0),
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
            top_rect(win_h, 18.0, 13.0, 120.0, 22.0),
            "设置",
            15.0,
            true,
            false,
        );
        settings_title_label.setHidden(true);

        // 右上角功能区按钮
        let pin_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 78.0, 10.0, 28.0, 28.0),
            "pin",
            "📌",
            "固定窗口",
            ActionTag::TogglePin as isize,
            dispatcher,
            action_sel,
        );
        let new_chat_top_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 110.0, 10.0, 28.0, 28.0),
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
            top_rect(win_h, win_w - 78.0, 10.0, 28.0, 28.0),
            "clock.arrow.circlepath",
            "⏱",
            "展开/收起对话历史条",
            ActionTag::ToggleHistoryBar as isize,
            dispatcher,
            action_sel,
        );
        set_button_symbol(&history_top_btn, "clock.arrow.circlepath", "⏱", true);
        history_top_btn.setHidden(true);

        let gear_btn = make_symbol_button(
            mtm,
            top_rect(win_h, win_w - 44.0, 10.0, 28.0, 28.0),
            "gearshape",
            "⚙",
            "设置",
            ActionTag::TabSettings as isize,
            dispatcher,
            action_sel,
        );

        let done_settings_btn = make_flat_button(
            mtm,
            top_rect(win_h, win_w - 60.0, 11.0, 44.0, 26.0),
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
        let chat_ui = Self::build_chat_view(mtm, body_frame, &pane_layout, dispatcher, action_sel);
        let settings_ui =
            Self::build_settings_view(mtm, body_frame, &settings, dispatcher, action_sel);

        chat_ui.root.setHidden(true);
        settings_ui.root.setHidden(true);

        add_subview(&effect_view, &translate_ui.root);
        add_subview(&effect_view, &chat_ui.root);
        add_subview(&effect_view, &settings_ui.root);

        let sc_cfg = settings.shortcuts();
        let mut registered_hotkeys = Vec::new();
        let mut parsed_shortcuts = ParsedShortcuts::default();
        let shortcut_result = parse_shortcut_config(&sc_cfg)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                register_shortcuts(&hotkey_mgr, &mut registered_hotkeys, &parsed)?;
                parsed_shortcuts = parsed;
                Ok(())
            });

        let conversations = history.read_conversations().unwrap_or_default();
        let (active_conversation_id, current_messages) = match conversations.last() {
            Some(last) => (Some(last.id), last.messages.clone()),
            None => (None, Vec::new()),
        };

        let mut ctrl = Self {
            updater: Default::default(),
            update_view: Default::default(),
            mtm,
            rt,
            ui_tx,
            settings,
            history,
            hotkey_mgr,
            parsed_shortcuts,
            registered_hotkeys,
            recording_shortcut: false,
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
            show_chat_history_bar: true,
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
            hymt_cancel: CancelSlot::new(),
            hymt_downloading: false,
            last_hymt_report: None,
            pending_clear_ai_key: false,
            pending_clear_chat_key: false,
            pane_layout,
            is_programmatic_resize: false,
            last_known_size: (win_w, win_h),
            pending_resize_save: None,
            ink_tone: initial_tone,
            target_ink_tone: initial_tone,
            last_backdrop_sample: None,
        };

        ctrl.refresh_hymt_ui_state();
        ctrl.update_window_min_max_constraints();
        ctrl.refresh_ai_header_badge();
        ctrl.refresh_conversations_popup();
        ctrl.relayout_all(win_w, win_h);
        if let Err(error) = shortcut_result {
            ctrl.set_status(&error);
        }
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
        let source_surface = make_surface_box(mtm, top_rect(h, 0.0, 0.0, w, source_h), 0.0, 0.25);
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
        let lang_surface = make_surface_box(mtm, top_rect(h, 0.0, source_h, w, lang_h), 0.0, 0.35);
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
            top_rect(
                h,
                2.0,
                results_top + 22.0,
                w - 4.0,
                (web_h - 26.0).max(40.0),
            ),
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

        // 顶部对话历史选取条（默认展示，也可点击右上角时钟按钮展开/收起）
        let hist_h = 38.0;
        let history_bar = NSView::initWithFrame(mtm.alloc(), top_rect(h, 0.0, 0.0, w, hist_h));
        let history_surface =
            make_surface_box(mtm, top_rect(hist_h, 0.0, 0.0, w, hist_h), 0.0, 0.28);
        let history_label = make_label(
            mtm,
            top_rect(hist_h, 14.0, 10.0, 56.0, 18.0),
            "历史对话",
            12.0,
            false,
            true,
        );
        let conv_popup = make_popup(
            mtm,
            top_rect(hist_h, 72.0, 6.0, (w - 216.0).max(120.0), 25.0),
            &["● 当前新对话"],
            0,
            true,
            ActionTag::SelectConversation as isize,
            dispatcher,
            action_sel,
        );
        let new_conv_btn = make_button(
            mtm,
            top_rect(hist_h, w - 136.0, 7.0, 64.0, 24.0),
            "＋ 新对话",
            ActionTag::NewConversation as isize,
            dispatcher,
            action_sel,
        );
        let clear_hist_btn = make_button(
            mtm,
            top_rect(hist_h, w - 66.0, 7.0, 54.0, 24.0),
            "清空",
            ActionTag::ClearChatHistory as isize,
            dispatcher,
            action_sel,
        );
        let history_bottom_line = make_divider(mtm, top_rect(hist_h, 0.0, hist_h - 1.0, w, 1.0));
        add_subview(&history_bar, &history_surface);
        add_subview(&history_bar, &history_label);
        add_subview(&history_bar, &conv_popup);
        add_subview(&history_bar, &new_conv_btn);
        add_subview(&history_bar, &clear_hist_btn);
        add_subview(&history_bar, &history_bottom_line);
        history_bar.setHidden(false);

        let composer_h = layout.chat_input_height;
        let transcript_h = (h - hist_h - composer_h).max(120.0);
        let transcript_scroll =
            NSScrollView::initWithFrame(mtm.alloc(), top_rect(h, 0.0, hist_h, w, transcript_h));
        transcript_scroll.setHasVerticalScroller(true);
        transcript_scroll.setHasHorizontalScroller(false);
        transcript_scroll.setDrawsBackground(false);
        let transcript_doc = make_flipped_view(
            mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, transcript_h)),
        );
        transcript_scroll.setDocumentView(Some(&transcript_doc));

        let composer_top = hist_h + transcript_h;
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
            top_rect(
                h,
                6.0,
                composer_top + 6.0,
                w - 88.0,
                (composer_h - 12.0).max(40.0),
            ),
            true,
            14.0,
            Some(dispatcher),
        );
        let input_placeholder = make_label(
            mtm,
            top_rect(h, 20.0, composer_top + 15.0, 220.0, 20.0),
            "输入消息，回车发送 (Shift+Enter 换行)",
            13.5,
            false,
            true,
        );
        let send_btn = make_button(
            mtm,
            top_rect(
                h,
                w - 74.0,
                composer_top + (composer_h - 28.0) * 0.5,
                62.0,
                28.0,
            ),
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
            history_surface,
            history_label,
            conv_popup,
            new_conv_btn,
            clear_hist_btn,
            history_bottom_line,
            transcript_scroll,
            transcript_doc,
            composer_split_bar,
            composer_surface,
            input_scroll,
            input_tv,
            input_placeholder,
            send_btn,
        }
    }

    /// 构建原版左侧分类栏 (`AI 翻译` / `AI 对话` / `快捷键` / `通用`) + 右侧自适应宽度与动态字段的设置面板。
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
            top_rect(h, 10.0, 14.0, sidebar_w - 20.0, 32.0),
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
            top_rect(h, 10.0, 50.0, sidebar_w - 20.0, 32.0),
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
            top_rect(h, 10.0, 86.0, sidebar_w - 20.0, 32.0),
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
            top_rect(h, 10.0, 122.0, sidebar_w - 20.0, 32.0),
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

        let content_x = sidebar_w + 18.0;
        let content_w = (w - content_x - 18.0).max(300.0);
        let sec_frame = top_rect(h, content_x, 8.0, content_w, (h - 38.0).max(260.0));
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

        let quant_labels: Vec<&str> = HyMtQuant::ALL.iter().map(|q| q.label()).collect();
        let ai_quant_idx = HyMtQuant::ALL
            .iter()
            .position(|&q| q == ai_cfg.hy_mt_quant)
            .unwrap_or(6);
        let chat_quant_idx = HyMtQuant::ALL
            .iter()
            .position(|&q| q == chat_cfg.hy_mt_quant)
            .unwrap_or(6);

        let dummy = top_rect(sec_h, 0.0, 0.0, 100.0, 24.0);

        // ── Section 1: AI 翻译（根据所选接入方式动态展示对应字段，多行提示词大框） ──
        let t_head = make_label(
            mtm,
            dummy,
            "AI 翻译（内置 HY-MT1.5 / 云端 / 本地模型）",
            12.5,
            true,
            true,
        );
        let ai_enabled_chk = make_checkbox(
            mtm,
            dummy,
            "启用 AI 翻译",
            ai_cfg.enabled,
            0,
            dispatcher,
            action_sel,
        );
        let t_lbl_backend = make_label(mtm, dummy, "接入方式", 12.5, false, true);
        let ai_backend_popup = make_popup(
            mtm,
            dummy,
            &backend_labels,
            ai_backend_idx,
            true,
            ActionTag::AiTransBackendChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_ai_def_url = make_button(
            mtm,
            dummy,
            "默认地址",
            ActionTag::AiTransFillDefaultUrl as isize,
            dispatcher,
            action_sel,
        );

        let t_lbl_hymt_quant = make_label(mtm, dummy, "模型精度", 12.5, false, true);
        let ai_hymt_quant_popup = make_popup(
            mtm,
            dummy,
            &quant_labels,
            ai_quant_idx,
            true,
            ActionTag::HyMtQuantChanged as isize,
            dispatcher,
            action_sel,
        );
        let t_lbl_hymt_action = make_label(mtm, dummy, "服务操作", 12.5, false, true);
        let btn_hymt_check = make_button(
            mtm,
            dummy,
            "检测环境",
            ActionTag::HyMtCheckCompatibility as isize,
            dispatcher,
            action_sel,
        );
        let btn_hymt_download_run = make_button(
            mtm,
            dummy,
            "下载并运行",
            ActionTag::HyMtDownloadAndRun as isize,
            dispatcher,
            action_sel,
        );
        let btn_hymt_stop = make_button(
            mtm,
            dummy,
            "停止",
            ActionTag::HyMtStopServer as isize,
            dispatcher,
            action_sel,
        );
        let btn_hymt_delete = make_button(
            mtm,
            dummy,
            "删除",
            ActionTag::HyMtDeleteModel as isize,
            dispatcher,
            action_sel,
        );
        let t_lbl_hymt_status = make_label(mtm, dummy, "环境状态", 12.5, false, true);
        let hymt_info_surface = make_surface_box(mtm, dummy, 6.0, 0.35);
        let hymt_info_label = make_wrapping_label(
            mtm,
            dummy,
            "点击「检测环境」检查内存与磁盘，或点击「下载并运行」从 Hugging Face 获取模型。",
            11.0,
            true,
        );

        let t_lbl_url = make_label(mtm, dummy, "接口地址", 12.5, false, true);
        let ai_base_url_field = make_text_field(
            mtm,
            dummy,
            &ai_cfg.base_url,
            "https://api.openai.com/v1 或 http://127.0.0.1:11434",
        );

        let t_lbl_key = make_label(mtm, dummy, "API 密钥", 12.5, false, true);
        let ai_key_field = make_secure_field(
            mtm,
            dummy,
            if ai_cfg.has_api_key {
                "已保存，留空保持不变"
            } else {
                "sk-… (保存在本地 JSON，不弹窗)"
            },
        );
        let ai_key_status_label = make_label(
            mtm,
            dummy,
            if ai_cfg.has_api_key {
                "已保存"
            } else {
                "未设置"
            },
            11.5,
            false,
            true,
        );
        set_label_alignment_right(&ai_key_status_label);
        let btn_ai_clr_key = make_button(
            mtm,
            dummy,
            "清除",
            ActionTag::AiTransClearKey as isize,
            dispatcher,
            action_sel,
        );

        let t_lbl_model = make_label(mtm, dummy, "模型名称", 12.5, false, true);
        let ai_model_field = make_text_field(
            mtm,
            dummy,
            &ai_cfg.model,
            "gpt-4o-mini / qwen2.5:7b / deepseek-chat",
        );
        let btn_ai_discover = make_button(
            mtm,
            dummy,
            "探测模型",
            ActionTag::AiTransDiscoverModels as isize,
            dispatcher,
            action_sel,
        );
        let ai_discovered_popup = make_popup(
            mtm,
            dummy,
            &["选择…"],
            0,
            true,
            ActionTag::AiTransModelPopupSelected as isize,
            dispatcher,
            action_sel,
        );

        let t_lbl_gguf = make_label(mtm, dummy, "GGUF模型", 12.5, false, true);
        let ai_gguf_field = make_text_field(
            mtm,
            dummy,
            &ai_cfg.gguf_model_path,
            "选择本地 .gguf 量化模型文件路径",
        );
        let btn_ai_gguf = make_button(
            mtm,
            dummy,
            "浏览…",
            ActionTag::AiTransBrowseGguf as isize,
            dispatcher,
            action_sel,
        );

        let t_lbl_tok = make_label(mtm, dummy, "分词词表", 12.5, false, true);
        let ai_tokenizer_field = make_text_field(
            mtm,
            dummy,
            &ai_cfg.gguf_tokenizer_path,
            "可选 tokenizer.json (留空则自动读取 GGUF 内嵌词表)",
        );
        let btn_ai_tok = make_button(
            mtm,
            dummy,
            "浏览…",
            ActionTag::AiTransBrowseTokenizer as isize,
            dispatcher,
            action_sel,
        );

        let t_lbl_prompt = make_label(
            mtm,
            dummy,
            "翻译提示词（可用 {source} 与 {target} 占位符）",
            12.0,
            false,
            true,
        );
        let ai_prompt_input = make_multiline_input(
            mtm,
            top_rect(sec_h, 0.0, 180.0, content_w, 92.0),
            &ai_cfg.translate_prompt,
            12.5,
        );
        let save_ai_btn = make_button(
            mtm,
            dummy,
            "保存配置",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );

        add_subview(&sec_translate, &t_head);
        add_subview(&sec_translate, &ai_enabled_chk);
        add_subview(&sec_translate, &t_lbl_backend);
        add_subview(&sec_translate, &ai_backend_popup);
        add_subview(&sec_translate, &btn_ai_def_url);
        add_subview(&sec_translate, &t_lbl_hymt_quant);
        add_subview(&sec_translate, &ai_hymt_quant_popup);
        add_subview(&sec_translate, &t_lbl_hymt_action);
        add_subview(&sec_translate, &btn_hymt_check);
        add_subview(&sec_translate, &btn_hymt_download_run);
        add_subview(&sec_translate, &btn_hymt_stop);
        add_subview(&sec_translate, &btn_hymt_delete);
        add_subview(&sec_translate, &t_lbl_hymt_status);
        add_subview(&sec_translate, &hymt_info_surface);
        add_subview(&sec_translate, &hymt_info_label);
        add_subview(&sec_translate, &t_lbl_url);
        add_subview(&sec_translate, &ai_base_url_field);
        add_subview(&sec_translate, &t_lbl_key);
        add_subview(&sec_translate, &ai_key_field);
        add_subview(&sec_translate, &ai_key_status_label);
        add_subview(&sec_translate, &btn_ai_clr_key);
        add_subview(&sec_translate, &t_lbl_model);
        add_subview(&sec_translate, &ai_model_field);
        add_subview(&sec_translate, &btn_ai_discover);
        add_subview(&sec_translate, &ai_discovered_popup);
        add_subview(&sec_translate, &t_lbl_gguf);
        add_subview(&sec_translate, &ai_gguf_field);
        add_subview(&sec_translate, &btn_ai_gguf);
        add_subview(&sec_translate, &t_lbl_tok);
        add_subview(&sec_translate, &ai_tokenizer_field);
        add_subview(&sec_translate, &btn_ai_tok);
        add_subview(&sec_translate, &t_lbl_prompt);
        add_subview(&sec_translate, &ai_prompt_input.surface);
        add_subview(&sec_translate, &ai_prompt_input.scroll);
        add_subview(&sec_translate, &save_ai_btn);

        // ── Section 2: AI 对话（根据接入方式动态展示对应字段，多行系统提示词大框） ──
        let c_head = make_label(
            mtm,
            dummy,
            "AI 对话（独立模型与系统提示词）",
            12.5,
            true,
            true,
        );
        let btn_sync_from_trans = make_button(
            mtm,
            dummy,
            "复用 AI 翻译配置",
            ActionTag::ChatCopyFromTrans as isize,
            dispatcher,
            action_sel,
        );
        let c_lbl_backend = make_label(mtm, dummy, "接入方式", 12.5, false, true);
        let chat_backend_popup = make_popup(
            mtm,
            dummy,
            &backend_labels,
            chat_backend_idx,
            true,
            ActionTag::ChatBackendChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_chat_def_url = make_button(
            mtm,
            dummy,
            "默认地址",
            ActionTag::ChatFillDefaultUrl as isize,
            dispatcher,
            action_sel,
        );
        let c_lbl_hymt_quant = make_label(mtm, dummy, "模型精度", 12.5, false, true);
        let chat_hymt_quant_popup = make_popup(
            mtm,
            dummy,
            &quant_labels,
            chat_quant_idx,
            true,
            ActionTag::ChatHyMtQuantChanged as isize,
            dispatcher,
            action_sel,
        );
        let btn_chat_hymt_run = make_button(
            mtm,
            dummy,
            "下载并运行",
            ActionTag::ChatHyMtDownloadAndRun as isize,
            dispatcher,
            action_sel,
        );

        let c_lbl_url = make_label(mtm, dummy, "接口地址", 12.5, false, true);
        let chat_base_url_field = make_text_field(
            mtm,
            dummy,
            &chat_cfg.base_url,
            "https://api.openai.com/v1 或 http://127.0.0.1:11434",
        );

        let c_lbl_key = make_label(mtm, dummy, "API 密钥", 12.5, false, true);
        let chat_key_field = make_secure_field(
            mtm,
            dummy,
            if chat_cfg.has_api_key {
                "已保存，留空保持不变"
            } else {
                "sk-… (保存在本地 JSON，不弹窗)"
            },
        );
        let chat_key_status_label = make_label(
            mtm,
            dummy,
            if chat_cfg.has_api_key {
                "已保存"
            } else {
                "未设置"
            },
            11.5,
            false,
            true,
        );
        set_label_alignment_right(&chat_key_status_label);
        let btn_chat_clr_key = make_button(
            mtm,
            dummy,
            "清除",
            ActionTag::ChatClearKey as isize,
            dispatcher,
            action_sel,
        );

        let c_lbl_model = make_label(mtm, dummy, "模型名称", 12.5, false, true);
        let chat_model_field = make_text_field(
            mtm,
            dummy,
            &chat_cfg.model,
            "gpt-4o-mini / qwen2.5:7b / deepseek-chat",
        );
        let btn_chat_discover = make_button(
            mtm,
            dummy,
            "探测模型",
            ActionTag::ChatDiscoverModels as isize,
            dispatcher,
            action_sel,
        );
        let chat_discovered_popup = make_popup(
            mtm,
            dummy,
            &["选择…"],
            0,
            true,
            ActionTag::ChatModelPopupSelected as isize,
            dispatcher,
            action_sel,
        );

        let c_lbl_gguf = make_label(mtm, dummy, "GGUF模型", 12.5, false, true);
        let chat_gguf_field = make_text_field(
            mtm,
            dummy,
            &chat_cfg.gguf_model_path,
            "选择本地 .gguf 量化模型文件路径",
        );
        let btn_chat_gguf = make_button(
            mtm,
            dummy,
            "浏览…",
            ActionTag::ChatBrowseGguf as isize,
            dispatcher,
            action_sel,
        );

        let c_lbl_prompt = make_label(
            mtm,
            dummy,
            "系统提示词（可选，作为多轮对话开头的 System Prompt）",
            12.0,
            false,
            true,
        );
        let chat_prompt_input = make_multiline_input(
            mtm,
            top_rect(sec_h, 0.0, 180.0, content_w, 96.0),
            &chat_cfg.chat_prompt,
            12.5,
        );
        let save_chat_btn = make_button(
            mtm,
            dummy,
            "保存配置",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );

        add_subview(&sec_chat, &c_head);
        add_subview(&sec_chat, &btn_sync_from_trans);
        add_subview(&sec_chat, &c_lbl_backend);
        add_subview(&sec_chat, &chat_backend_popup);
        add_subview(&sec_chat, &btn_chat_def_url);
        add_subview(&sec_chat, &c_lbl_hymt_quant);
        add_subview(&sec_chat, &chat_hymt_quant_popup);
        add_subview(&sec_chat, &btn_chat_hymt_run);
        add_subview(&sec_chat, &c_lbl_url);
        add_subview(&sec_chat, &chat_base_url_field);
        add_subview(&sec_chat, &c_lbl_key);
        add_subview(&sec_chat, &chat_key_field);
        add_subview(&sec_chat, &chat_key_status_label);
        add_subview(&sec_chat, &btn_chat_clr_key);
        add_subview(&sec_chat, &c_lbl_model);
        add_subview(&sec_chat, &chat_model_field);
        add_subview(&sec_chat, &btn_chat_discover);
        add_subview(&sec_chat, &chat_discovered_popup);
        add_subview(&sec_chat, &c_lbl_gguf);
        add_subview(&sec_chat, &chat_gguf_field);
        add_subview(&sec_chat, &btn_chat_gguf);
        add_subview(&sec_chat, &c_lbl_prompt);
        add_subview(&sec_chat, &chat_prompt_input.surface);
        add_subview(&sec_chat, &chat_prompt_input.scroll);
        add_subview(&sec_chat, &save_chat_btn);

        // ── Section 3: 快捷键 ──
        let sc_head = make_label(mtm, dummy, "全局快捷键", 12.5, true, true);
        let btn_reset_sc = make_button(
            mtm,
            dummy,
            "恢复默认热键",
            ActionTag::ResetDefaultShortcuts as isize,
            dispatcher,
            action_sel,
        );
        let sc_lbl_toggle = make_label(mtm, dummy, "呼出翻译", 12.5, false, true);
        let sc_toggle_field = ShortcutField::new(mtm, dummy, &sc_cfg.toggle);
        let sc_lbl_ocr = make_label(mtm, dummy, "截图 OCR", 12.5, false, true);
        let sc_ocr_field = ShortcutField::new(mtm, dummy, &sc_cfg.ocr);
        let sc_lbl_ai = make_label(mtm, dummy, "呼出 AI 对话", 12.5, false, true);
        let sc_ai_field = ShortcutField::new(mtm, dummy, &sc_cfg.ai_dialog);
        let sc_lbl_sel_trans = make_label(mtm, dummy, "划词翻译", 12.5, false, true);
        let sc_sel_trans_field = ShortcutField::new(mtm, dummy, &sc_cfg.selection_translate);
        let sc_lbl_sel_ai = make_label(mtm, dummy, "划词 AI 对话", 12.5, false, true);
        let sc_sel_ai_field = ShortcutField::new(mtm, dummy, &sc_cfg.selection_ai_dialog);
        let save_sc_btn = make_button(
            mtm,
            dummy,
            "保存快捷键",
            ActionTag::SaveSettings as isize,
            dispatcher,
            action_sel,
        );
        let sc_hint = make_wrapping_label(
            mtm,
            top_rect(sec_h, 0.0, 260.0, content_w, 60.0),
            "点击后按组合键；Delete 清空，Esc 取消，Tab 切换。保存后生效。呼出与对应划词操作可共用快捷键：选中文本时执行划词，否则切换窗口。",
            11.5,
            true,
        );

        add_subview(&sec_shortcuts, &sc_head);
        add_subview(&sec_shortcuts, &btn_reset_sc);
        add_subview(&sec_shortcuts, &sc_lbl_toggle);
        add_subview(&sec_shortcuts, &sc_toggle_field);
        add_subview(&sec_shortcuts, &sc_lbl_ocr);
        add_subview(&sec_shortcuts, &sc_ocr_field);
        add_subview(&sec_shortcuts, &sc_lbl_ai);
        add_subview(&sec_shortcuts, &sc_ai_field);
        add_subview(&sec_shortcuts, &sc_lbl_sel_trans);
        add_subview(&sec_shortcuts, &sc_sel_trans_field);
        add_subview(&sec_shortcuts, &sc_lbl_sel_ai);
        add_subview(&sec_shortcuts, &sc_sel_ai_field);
        add_subview(&sec_shortcuts, &save_sc_btn);
        add_subview(&sec_shortcuts, &sc_hint);

        // ── Section 4: 通用 ──
        let gen_head = make_label(mtm, dummy, "通用与外观", 12.5, true, true);
        let gen_lbl_opacity = make_label(mtm, dummy, "面板透明度", 12.5, false, false);
        let opacity_val_label = make_label(
            mtm,
            dummy,
            &format!("{:.0}%", app_cfg.panel_opacity * 100.0),
            12.0,
            true,
            true,
        );
        set_label_alignment_right(&opacity_val_label);
        let opacity_slider = make_slider(
            mtm,
            dummy,
            PANEL_OPACITY_MIN,
            PANEL_OPACITY_MAX,
            app_cfg.panel_opacity,
            ActionTag::OpacitySliderChanged as isize,
            dispatcher,
            action_sel,
        );
        let autostart_chk = make_checkbox(
            mtm,
            dummy,
            "开机自动启动",
            ttree_platform::autostart::is_enabled(),
            ActionTag::AutostartToggled as isize,
            dispatcher,
            action_sel,
        );
        let ocr_strip_newlines_chk = make_checkbox(
            mtm,
            dummy,
            "OCR 识别文字自动去掉换行（智能合并为单段）",
            app_cfg.ocr_strip_newlines,
            ActionTag::OcrStripNewlinesToggled as isize,
            dispatcher,
            action_sel,
        );
        let update_button = make_flat_button(
            mtm,
            dummy,
            "检查更新",
            12.0,
            false,
            false,
            ActionTag::UpdateApp as isize,
            dispatcher,
            action_sel,
        );
        let update_status = make_wrapping_label(
            mtm,
            dummy,
            &ttree_updater::View::default().status,
            11.5,
            true,
        );
        add_subview(&sec_general, &update_button);
        add_subview(&sec_general, &update_status);
        let ver_label = make_label(
            mtm,
            dummy,
            concat!("版本：", env!("CARGO_PKG_VERSION"), " (Native AppKit)"),
            12.0,
            false,
            true,
        );
        let gen_hint = make_wrapping_label(
            mtm,
            top_rect(sec_h, 0.0, 204.0, content_w, 88.0),
            "调整面板透明度可让底层桌面毛玻璃透出。\n截图 OCR 需要 macOS「屏幕录制」权限；划词翻译优先使用「辅助功能」权限，未授权时会临时使用剪贴板回退。\n所有配置与分栏尺寸均保存在本地 JSON，不再调用 macOS 钥匙串。",
            11.5,
            true,
        );
        add_subview(&sec_general, &gen_head);
        add_subview(&sec_general, &gen_lbl_opacity);
        add_subview(&sec_general, &opacity_val_label);
        add_subview(&sec_general, &opacity_slider);
        add_subview(&sec_general, &autostart_chk);
        add_subview(&sec_general, &ocr_strip_newlines_chk);
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
            t_head,
            ai_enabled_chk,
            t_lbl_backend,
            ai_backend_popup,
            btn_ai_def_url,
            t_lbl_hymt_quant,
            ai_hymt_quant_popup,
            t_lbl_hymt_action,
            btn_hymt_check,
            btn_hymt_download_run,
            btn_hymt_stop,
            btn_hymt_delete,
            t_lbl_hymt_status,
            hymt_info_surface,
            hymt_info_label,
            t_lbl_url,
            ai_base_url_field,
            t_lbl_key,
            ai_key_field,
            ai_key_status_label,
            btn_ai_clr_key,
            t_lbl_model,
            ai_model_field,
            btn_ai_discover,
            ai_discovered_popup,
            t_lbl_gguf,
            ai_gguf_field,
            btn_ai_gguf,
            t_lbl_tok,
            ai_tokenizer_field,
            btn_ai_tok,
            t_lbl_prompt,
            ai_prompt_input,
            save_ai_btn,
            c_head,
            btn_sync_from_trans,
            c_lbl_backend,
            chat_backend_popup,
            btn_chat_def_url,
            c_lbl_hymt_quant,
            chat_hymt_quant_popup,
            btn_chat_hymt_run,
            c_lbl_url,
            chat_base_url_field,
            c_lbl_key,
            chat_key_field,
            chat_key_status_label,
            btn_chat_clr_key,
            c_lbl_model,
            chat_model_field,
            btn_chat_discover,
            chat_discovered_popup,
            c_lbl_gguf,
            chat_gguf_field,
            btn_chat_gguf,
            c_lbl_prompt,
            chat_prompt_input,
            save_chat_btn,
            sc_head,
            btn_reset_sc,
            sc_lbl_toggle,
            sc_toggle_field,
            sc_lbl_ocr,
            sc_ocr_field,
            sc_lbl_ai,
            sc_ai_field,
            sc_lbl_sel_trans,
            sc_sel_trans_field,
            sc_lbl_sel_ai,
            sc_sel_ai_field,
            save_sc_btn,
            sc_hint,
            gen_head,
            gen_lbl_opacity,
            opacity_slider,
            opacity_val_label,
            autostart_chk,
            ocr_strip_newlines_chk,
            update_button,
            update_status,
            ver_label,
            gen_hint,
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
            TabMode::Translate | TabMode::Chat => {
                self.window.setMinSize(NSSize::new(
                    WINDOW_MIN_WIDTH,
                    PaneLayoutConfig::min_window_height(ai_enabled),
                ));
                self.window.setMaxSize(NSSize::new(
                    WINDOW_MAX_WIDTH,
                    PaneLayoutConfig::max_window_height(ai_enabled),
                ));
            }
            TabMode::Settings => {
                self.window.setMinSize(NSSize::new(480.0, 470.0));
                self.window
                    .setMaxSize(NSSize::new(WINDOW_MAX_WIDTH, WINDOW_MAX_HEIGHT));
            }
        }
    }

    /// 将窗口尺寸严格同步至记忆的 `pane_layout`（翻译页与 AI 对话页宽高完全共享一致，顶边在屏幕上的位置保持固定不动）。
    pub fn sync_window_to_pane_layout(&mut self) {
        self.pane_layout = self.pane_layout.clamped();
        let ai_enabled = self.settings.ai_translation_for_request().enabled;
        self.update_window_min_max_constraints();

        let shared_w = self.pane_layout.width;
        let shared_h = self.pane_layout.translate_window_height(ai_enabled);
        let (target_w, target_h) = match self.active_tab {
            TabMode::Translate | TabMode::Chat => (shared_w, shared_h),
            TabMode::Settings => (shared_w.max(480.0), shared_h.max(480.0)),
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
                let _ = self
                    .window
                    .makeFirstResponder(Some(&self.translate_ui.input_tv));
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
        self.sample_backdrop_and_step_ink(true);
        match self.active_tab {
            TabMode::Translate => {
                let _ = self
                    .window
                    .makeFirstResponder(Some(&self.translate_ui.input_tv));
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
        let short_model = cfg
            .model
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&cfg.model);
        let label = if !cfg.enabled {
            "AI (未开启)".to_string()
        } else {
            match cfg.backend {
                AiBackendKind::BuiltInHyMt => {
                    format!("AI  ·  HY-MT1.5 ({})", cfg.hy_mt_quant.code())
                }
                AiBackendKind::OpenAiCompatible => format!("AI  ·  {short_model}"),
                AiBackendKind::Ollama => format!("AI  ·  OLLAMA ({short_model})"),
                AiBackendKind::LocalServer => format!("AI  ·  MLX/LOCAL ({short_model})"),
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
        let (resolved_src, resolved_tgt) = resolve_language_pair(src_ui, tgt_ui, &trimmed);

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
        #[cfg(feature = "updater-smoke")]
        let evidence = {
            let args: Vec<_> = std::env::args_os().collect();
            args.windows(2)
                .find(|pair| pair[0] == "--ocr-evidence")
                .map(|pair| {
                    let path = std::path::PathBuf::from(&pair[1]);
                    let _ = std::fs::write(
                        &path,
                        format!(
                            "preflight={}\n",
                            ttree_ocr::screenshot::has_screen_capture_permission()
                        ),
                    );
                    path
                })
        };
        let tx = self.ui_tx.clone();
        self.rt.spawn(async move {
            let res = tokio::task::spawn_blocking(ttree_ocr::capture_and_recognize)
                .await
                .unwrap_or_else(|e| Err(format!("OCR 线程异常: {e}")));
            #[cfg(feature = "updater-smoke")]
            if let Some(path) = evidence {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
                    let _ = writeln!(file, "{res:?}");
                }
            }
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
                        ttree_platform::selection::capture_selected_text,
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
                        ttree_platform::selection::capture_selected_text,
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
                self.refresh_hymt_ui_state();
                self.switch_tab(TabMode::Settings);
                self.show_and_focus_window();
            }
            ActionTag::CloseSettings => {
                self.switch_tab(self.last_main_mode);
            }
            ActionTag::SettingsSecTranslate => {
                self.refresh_hymt_ui_state();
                self.switch_settings_section(SettingsSection::Translate);
            }
            ActionTag::SettingsSecChat => {
                self.refresh_hymt_ui_state();
                self.switch_settings_section(SettingsSection::Chat);
            }
            ActionTag::SettingsSecShortcuts => {
                self.switch_settings_section(SettingsSection::Shortcuts);
            }
            ActionTag::UpdateApp => {
                self.updater.activate(&self.rt);
            }
            ActionTag::SettingsSecGeneral => {
                self.switch_settings_section(SettingsSection::General);
            }
            ActionTag::TriggerOcr => self.trigger_ocr(),
            ActionTag::TogglePin => self.set_pinned(!self.is_pinned),
            ActionTag::TrayQuit => {
                ttree_ai::hy_mt_manager::stop_server();
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
                    self.refresh_conversations_popup();
                    self.render_chat_transcript();
                } else if idx >= 0 && (idx as usize) == self.conversations.len() {
                    self.active_conversation_id = None;
                    self.current_messages.clear();
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
                if !matches!(
                    backend,
                    AiBackendKind::BuiltInHyMt | AiBackendKind::CandleGguf
                ) {
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
                self.refresh_hymt_ui_state();
                let (w, h) = self.last_known_size;
                self.relayout_settings_view(w, (h - TOP_BAR_HEIGHT).max(200.0));
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
                    let gguf_idx = AiBackendKind::ALL
                        .iter()
                        .position(|&b| b == AiBackendKind::CandleGguf)
                        .unwrap_or(4);
                    self.settings_ui
                        .ai_backend_popup
                        .selectItemAtIndex(gguf_idx as isize);
                    let (w, h) = self.last_known_size;
                    self.relayout_settings_view(w, (h - TOP_BAR_HEIGHT).max(200.0));
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
                if !matches!(
                    backend,
                    AiBackendKind::BuiltInHyMt | AiBackendKind::CandleGguf
                ) {
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
                self.refresh_hymt_ui_state();
                let (w, h) = self.last_known_size;
                self.relayout_settings_view(w, (h - TOP_BAR_HEIGHT).max(200.0));
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
                    let gguf_idx = AiBackendKind::ALL
                        .iter()
                        .position(|&b| b == AiBackendKind::CandleGguf)
                        .unwrap_or(4);
                    self.settings_ui
                        .chat_backend_popup
                        .selectItemAtIndex(gguf_idx as isize);
                    let (w, h) = self.last_known_size;
                    self.relayout_settings_view(w, (h - TOP_BAR_HEIGHT).max(200.0));
                }
            }
            ActionTag::ChatCopyFromTrans => {
                let idx = self.settings_ui.ai_backend_popup.indexOfSelectedItem();
                self.settings_ui.chat_backend_popup.selectItemAtIndex(idx);
                let q_idx = self.settings_ui.ai_hymt_quant_popup.indexOfSelectedItem();
                self.settings_ui
                    .chat_hymt_quant_popup
                    .selectItemAtIndex(q_idx);
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
                let (w, h) = self.last_known_size;
                self.relayout_settings_view(w, (h - TOP_BAR_HEIGHT).max(200.0));
                self.set_status("✓ 已同步翻译模型配置，请点击保存");
            }
            ActionTag::ChatClearKey => {
                self.pending_clear_chat_key = true;
                set_field_string(&self.settings_ui.chat_key_field, "");
                set_field_string(&self.settings_ui.chat_key_status_label, "待清除");
            }
            ActionTag::ResetDefaultShortcuts => {
                let def = ShortcutConfig::default();
                self.settings_ui.sc_toggle_field.set_value(&def.toggle);
                self.settings_ui.sc_ocr_field.set_value(&def.ocr);
                self.settings_ui.sc_ai_field.set_value(&def.ai_dialog);
                self.settings_ui.sc_sel_trans_field.set_value(&def.selection_translate);
                self.settings_ui.sc_sel_ai_field.set_value(&def.selection_ai_dialog);
                self.set_status("已填入默认快捷键，请点击「保存快捷键」");
            }
            ActionTag::OpacitySliderChanged => {
                let val =
                    AppearanceConfig::clamp_opacity(self.settings_ui.opacity_slider.doubleValue());
                self.window.setAlphaValue(val);
                let mut app_cfg = self.settings.appearance();
                app_cfg.panel_opacity = val;
                let _ = self.settings.save_appearance(app_cfg);
                set_field_string(
                    &self.settings_ui.opacity_val_label,
                    &format!("{:.0}%", val * 100.0),
                );
            }
            ActionTag::AutostartToggled => {
                let want = self.settings_ui.autostart_chk.state() == NSControlStateValueOn;
                let _ = ttree_platform::autostart::set_enabled(want);
            }
            ActionTag::OcrStripNewlinesToggled => {
                let want = self.settings_ui.ocr_strip_newlines_chk.state() == NSControlStateValueOn;
                let mut app_cfg = self.settings.appearance();
                app_cfg.ocr_strip_newlines = want;
                let _ = self.settings.save_appearance(app_cfg);
                self.set_status(if want {
                    "✓ 已开启 OCR 自动去掉换行"
                } else {
                    "已关闭 OCR 自动去掉换行（保留原始多行格式）"
                });
            }
            ActionTag::SaveSettings => self.save_all_settings(),
            ActionTag::HyMtQuantChanged | ActionTag::ChatHyMtQuantChanged => {
                self.refresh_hymt_ui_state();
            }
            ActionTag::HyMtCheckCompatibility => {
                let report = self.refresh_hymt_ui_state();
                if report.supported {
                    self.set_status(&format!(
                        "✓ 环境支持 HY-MT1.5：{} ({:.0} GB 内存) · 推荐精度 {}",
                        report.cpu_brand,
                        report.total_ram_gb,
                        report.recommended_quant.code()
                    ));
                } else {
                    self.set_status(&format!(
                        "⚠ 环境资源紧张（内存 {:.1} GB / 可用磁盘 {:.1} GB），建议选择 Q2_K 轻量精度",
                        report.total_ram_gb, report.free_disk_gb
                    ));
                }
            }
            ActionTag::HyMtDownloadAndRun => {
                let quant = self.selected_ai_hymt_quant();
                self.trigger_hymt_download_and_run(quant, false);
            }
            ActionTag::ChatHyMtDownloadAndRun => {
                let quant = self.selected_chat_hymt_quant();
                self.trigger_hymt_download_and_run(quant, true);
            }
            ActionTag::HyMtStopServer => {
                ttree_ai::hy_mt_manager::stop_server();
                self.refresh_hymt_ui_state();
                self.set_status("✓ 已停止内置 HY-MT1.5 后台推理服务");
            }
            ActionTag::HyMtDeleteModel => {
                let quant = self.selected_ai_hymt_quant();
                match ttree_ai::hy_mt_manager::delete_model_quant(quant) {
                    Ok(()) => {
                        self.refresh_hymt_ui_state();
                        self.set_status(&format!(
                            "✓ 已删除本地模型 HY-MT1.5-1.8B ({})",
                            quant.code()
                        ));
                    }
                    Err(e) => {
                        self.set_status(&format!("❌ {e}"));
                    }
                }
            }
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

    fn selected_ai_hymt_quant(&self) -> HyMtQuant {
        let idx = self.settings_ui.ai_hymt_quant_popup.indexOfSelectedItem();
        HyMtQuant::ALL
            .get(idx.max(0) as usize)
            .copied()
            .unwrap_or(HyMtQuant::Q8_0)
    }

    fn selected_chat_hymt_quant(&self) -> HyMtQuant {
        let idx = self.settings_ui.chat_hymt_quant_popup.indexOfSelectedItem();
        HyMtQuant::ALL
            .get(idx.max(0) as usize)
            .copied()
            .unwrap_or(HyMtQuant::Q8_0)
    }

    pub fn refresh_hymt_ui_state(&mut self) -> ttree_ai::hy_mt_manager::HyMtCompatibilityReport {
        let report = ttree_ai::hy_mt_manager::check_compatibility();
        let ai_q = self.selected_ai_hymt_quant();
        let chat_q = self.selected_chat_hymt_quant();

        let rebuild_quant_popup = |popup: &NSPopUpButton, current: HyMtQuant| {
            popup.removeAllItems();
            for q in HyMtQuant::ALL {
                let tag = if report.running_quant == Some(q) {
                    " · 运行中"
                } else if report.downloaded_quants.contains(&q) {
                    " · 已下载"
                } else {
                    ""
                };
                let title = format!("{}{}", q.label(), tag);
                popup.addItemWithTitle(&NSString::from_str(&title));
            }
            if let Some(pos) = HyMtQuant::ALL.iter().position(|&q| q == current) {
                popup.selectItemAtIndex(pos as isize);
            }
        };

        rebuild_quant_popup(&self.settings_ui.ai_hymt_quant_popup, ai_q);
        rebuild_quant_popup(&self.settings_ui.chat_hymt_quant_popup, chat_q);

        if !self.hymt_downloading {
            let summary = report.summary_text(ai_q);
            set_field_string(&self.settings_ui.hymt_info_label, &summary);

            let dl_btn_title = if report.running_quant == Some(ai_q) {
                "● 运行中"
            } else if report.downloaded_quants.contains(&ai_q) {
                "启动服务"
            } else {
                "下载并运行"
            };
            self.settings_ui
                .btn_hymt_download_run
                .setTitle(&NSString::from_str(dl_btn_title));

            let chat_btn_title = if report.running_quant == Some(chat_q) {
                "● 运行中"
            } else if report.downloaded_quants.contains(&chat_q) {
                "启动服务"
            } else {
                "下载并运行"
            };
            self.settings_ui
                .btn_chat_hymt_run
                .setTitle(&NSString::from_str(chat_btn_title));
        } else {
            self.settings_ui
                .btn_hymt_download_run
                .setTitle(&NSString::from_str("取消下载"));
            self.settings_ui
                .btn_chat_hymt_run
                .setTitle(&NSString::from_str("取消下载"));
        }

        self.settings_ui
            .btn_hymt_stop
            .setEnabled(report.running_quant.is_some());
        self.settings_ui
            .btn_hymt_delete
            .setEnabled(report.downloaded_quants.contains(&ai_q) && !self.hymt_downloading);

        self.last_hymt_report = Some(report.clone());
        report
    }

    fn trigger_hymt_download_and_run(&mut self, quant: HyMtQuant, for_chat: bool) {
        if self.hymt_downloading {
            self.hymt_cancel.cancel_current();
            self.hymt_downloading = false;
            self.refresh_hymt_ui_state();
            self.set_status("已取消下载（断点进度已保留，再次点击可继续）");
            return;
        }

        // 若当前所选精度尚未在当前目录下载完成，先弹窗让用户选择/确认模型保存文件夹
        if !ttree_ai::hy_mt_manager::is_model_downloaded(quant) {
            let Some(chosen_dir) = self.pick_hymt_download_dir(quant) else {
                self.set_status("已取消选择模型下载文件夹");
                return;
            };
            if let Err(e) = ttree_ai::hy_mt_manager::set_custom_models_dir(chosen_dir) {
                self.set_status(&format!("❌ {e}"));
                return;
            }
            self.refresh_hymt_ui_state();
        }

        // 自动保存当前选择的内置模型与精度配置，无需用户额外点击“保存配置”
        if for_chat {
            let mut chat_cfg = self.settings.chat_ai_for_request();
            chat_cfg.backend = AiBackendKind::BuiltInHyMt;
            chat_cfg.hy_mt_quant = quant;
            let _ = self.settings.save_chat_ai(&chat_cfg);
        } else {
            let mut ai_cfg = self.settings.ai_translation_for_request();
            ai_cfg.enabled = true;
            self.settings_ui
                .ai_enabled_chk
                .setState(NSControlStateValueOn);
            ai_cfg.backend = AiBackendKind::BuiltInHyMt;
            ai_cfg.hy_mt_quant = quant;
            let _ = self.settings.save_ai_translation(&ai_cfg);
            self.refresh_ai_header_badge();
        }

        let cancel_token = self.hymt_cancel.next();
        self.hymt_downloading = true;
        self.refresh_hymt_ui_state();
        self.set_status(&format!(
            "正在准备 HY-MT1.5-1.8B ({}) → {}…",
            quant.code(),
            ttree_ai::hy_mt_manager::models_dir().display()
        ));

        let tx = self.ui_tx.clone();
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let res = ttree_ai::hy_mt_manager::download_and_start(quant, &cancel_token, move |p| {
                let _ = progress_tx.send(UiEvent::HyMtProgress { progress: p });
            })
            .await;
            let _ = tx.send(UiEvent::HyMtActionFinished { quant, result: res });
        });
    }

    fn discover_models(&mut self, for_chat: bool) {
        let (backend, base_url, api_key) = if for_chat {
            let typed_key = get_field_string(&self.settings_ui.chat_key_field);
            let key = if typed_key.trim().is_empty() {
                self.settings.chat_ai_for_request().api_key
            } else {
                typed_key
            };
            (
                self.selected_chat_backend(),
                get_field_string(&self.settings_ui.chat_base_url_field),
                key,
            )
        } else {
            let typed_key = get_field_string(&self.settings_ui.ai_key_field);
            let key = if typed_key.trim().is_empty() {
                self.settings.ai_translation_for_request().api_key
            } else {
                typed_key
            };
            (
                self.selected_ai_trans_backend(),
                get_field_string(&self.settings_ui.ai_base_url_field),
                key,
            )
        };
        self.set_status("正在探测本机模型…");
        let tx = self.ui_tx.clone();
        self.rt.spawn(async move {
            let opt_key = (!api_key.trim().is_empty()).then_some(api_key.trim());
            let res = ttree_ai::discover_local_models(backend, &base_url, opt_key).await;
            let _ = tx.send(UiEvent::ModelsDiscovered {
                for_chat,
                result: res,
            });
        });
    }

    fn pick_hymt_download_dir(&mut self, quant: HyMtQuant) -> Option<std::path::PathBuf> {
        self.is_modal_open = true;
        let panel = NSOpenPanel::openPanel(self.mtm);
        panel.setCanChooseFiles(false);
        panel.setCanChooseDirectories(true);
        panel.setCanCreateDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setTitle(Some(&NSString::from_str("选择模型下载保存文件夹")));
        let size_gb = quant.expected_bytes() as f64 / (1024.0 * 1024.0 * 1024.0);
        let msg = format!(
            "请选择存放混元翻译模型 {} ({:.2} GB) 的文件夹：",
            quant.file_name(),
            size_gb
        );
        panel.setMessage(Some(&NSString::from_str(&msg)));
        panel.setPrompt(Some(&NSString::from_str("选择此文件夹并下载")));

        let current_dir = ttree_ai::hy_mt_manager::models_dir();
        let _ = std::fs::create_dir_all(&current_dir);
        if let Some(dir_str) = current_dir.to_str() {
            let ns_path = NSString::from_str(dir_str);
            let url = objc2_foundation::NSURL::fileURLWithPath(&ns_path);
            panel.setDirectoryURL(Some(&url));
        }

        let response = panel.runModal();
        self.is_modal_open = false;
        if response != NSModalResponseOK {
            return None;
        }
        let urls = panel.URLs();
        let url = urls.firstObject()?;
        let path = url.path()?.to_string();
        Some(std::path::PathBuf::from(path))
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
        self.window.makeFirstResponder(None);
        self.set_shortcut_recording(false);
        let ai_input = AiTranslationConfig {
            enabled: self.settings_ui.ai_enabled_chk.state() == NSControlStateValueOn,
            backend: self.selected_ai_trans_backend(),
            hy_mt_quant: self.selected_ai_hymt_quant(),
            base_url: get_field_string(&self.settings_ui.ai_base_url_field),
            has_api_key: false,
            api_key: get_field_string(&self.settings_ui.ai_key_field),
            clear_api_key: self.pending_clear_ai_key,
            model: get_field_string(&self.settings_ui.ai_model_field),
            gguf_model_path: get_field_string(&self.settings_ui.ai_gguf_field),
            gguf_tokenizer_path: get_field_string(&self.settings_ui.ai_tokenizer_field),
            translate_prompt: get_text_view_string(&self.settings_ui.ai_prompt_input.tv),
        };
        if let Err(e) = self.settings.save_ai_translation(&ai_input) {
            self.set_status(&format!("❌ {e}"));
            return;
        }
        self.pending_clear_ai_key = false;

        let chat_input = ChatAiConfig {
            backend: self.selected_chat_backend(),
            hy_mt_quant: self.selected_chat_hymt_quant(),
            base_url: get_field_string(&self.settings_ui.chat_base_url_field),
            has_api_key: false,
            api_key: get_field_string(&self.settings_ui.chat_key_field),
            clear_api_key: self.pending_clear_chat_key,
            model: get_field_string(&self.settings_ui.chat_model_field),
            gguf_model_path: get_field_string(&self.settings_ui.chat_gguf_field),
            gguf_tokenizer_path: get_field_string(&self.settings_ui.ai_tokenizer_field),
            chat_prompt: get_text_view_string(&self.settings_ui.chat_prompt_input.tv),
        };
        if let Err(e) = self.settings.save_chat_ai(&chat_input) {
            self.set_status(&format!("❌ {e}"));
            return;
        }
        self.pending_clear_chat_key = false;

        let sc_input = ShortcutConfig {
            toggle: self.settings_ui.sc_toggle_field.value(),
            ocr: self.settings_ui.sc_ocr_field.value(),
            ai_dialog: self.settings_ui.sc_ai_field.value(),
            selection_translate: self.settings_ui.sc_sel_trans_field.value(),
            selection_ai_dialog: self.settings_ui.sc_sel_ai_field.value(),
        };
        match parse_shortcut_config(&sc_input) {
            Ok(parsed) => {
                if let Err(error) =
                    register_shortcuts(&self.hotkey_mgr, &mut self.registered_hotkeys, &parsed)
                {
                    self.set_status(&error);
                    return;
                }
                self.parsed_shortcuts = parsed;
                if let Err(error) = self.settings.save_shortcuts(&sc_input) {
                    self.set_status(&format!("保存快捷键失败: {error}"));
                    return;
                }
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
            if saved_ai.has_api_key {
                "已保存"
            } else {
                "未设置"
            },
        );
        set_field_string(&self.settings_ui.chat_key_field, "");
        set_field_string(
            &self.settings_ui.chat_key_status_label,
            if saved_chat.has_api_key {
                "已保存"
            } else {
                "未设置"
            },
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
                .addItemWithTitle(&NSString::from_str("● 当前新对话"));
            return;
        }
        for (i, conv) in self.conversations.iter().enumerate() {
            let msg_count = conv.messages.len();
            let summary = if conv.summary.is_empty() {
                format!("对话 #{}", i + 1)
            } else {
                conv.summary.clone()
            };
            let title = format!("{}. {}  ({}条)", i + 1, summary, msg_count);
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

    /// 渲染左（AI）/右（用户）分栏聊天气泡列表，类似信息对话排版。
    pub fn render_chat_transcript(&self) {
        clear_subviews(&self.chat_ui.transcript_doc);

        let content_size = self.chat_ui.transcript_scroll.contentSize();
        let scroll_w = content_size.width.max(260.0);
        let scroll_h = content_size.height.max(100.0);

        if self.current_messages.is_empty() && !self.chat_streaming {
            let empty_label = make_label(
                self.mtm,
                NSRect::new(
                    NSPoint::new(18.0, (scroll_h * 0.42).max(20.0)),
                    NSSize::new((scroll_w - 36.0).max(160.0), 22.0),
                ),
                "和 AI 自由对话（顶部可切换历史对话或开启新对话）",
                13.0,
                false,
                true,
            );
            add_subview(&self.chat_ui.transcript_doc, &empty_label);
            self.chat_ui.transcript_doc.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(scroll_w, scroll_h),
            ));
            return;
        }

        let max_bubble_w = (scroll_w * 0.78).clamp(180.0, 560.0);
        let pad_x = 12.0;
        let pad_y = 8.0;
        let max_text_w = (max_bubble_w - pad_x * 2.0).max(120.0);
        let mut y = 14.0;

        let mut render_bubble = |is_user: bool, raw_content: &str| {
            let role_w = 64.0;
            let role_h = 15.0;
            let role_x = if is_user {
                (scroll_w - 16.0 - role_w).max(16.0)
            } else {
                18.0
            };
            let role_label = make_label(
                self.mtm,
                NSRect::new(NSPoint::new(role_x, y), NSSize::new(role_w, role_h)),
                if is_user { "你" } else { "AI" },
                11.0,
                true,
                true,
            );
            if is_user {
                set_label_alignment_right(&role_label);
            }
            add_subview(&self.chat_ui.transcript_doc, &role_label);
            y += role_h + 3.0;

            let cleaned = clean_markdown_for_bubble(raw_content);
            let (msg_label, text_size) =
                make_wrapping_message_label(self.mtm, &cleaned, max_text_w, 13.5, false);

            let bubble_w = (text_size.width + pad_x * 2.0).clamp(44.0, max_bubble_w);
            let bubble_h = (text_size.height + pad_y * 2.0).max(34.0);
            let bubble_x = if is_user {
                (scroll_w - 16.0 - bubble_w).max(16.0)
            } else {
                16.0
            };

            let bubble_box = NSBox::initWithFrame(
                self.mtm.alloc(),
                NSRect::new(NSPoint::new(bubble_x, y), NSSize::new(bubble_w, bubble_h)),
            );
            bubble_box.setBoxType(NSBoxType::Custom);
            bubble_box.setCornerRadius(10.0);
            bubble_box.setContentViewMargins(CGSize::new(0.0, 0.0));
            if is_user {
                bubble_box.setBorderWidth(0.5);
                bubble_box.setBorderColor(&pale_ink(0.095));
                bubble_box.setFillColor(&pale_ink(0.075));
            } else {
                bubble_box.setBorderWidth(0.5);
                bubble_box.setBorderColor(&pale_ink(0.06));
                bubble_box.setFillColor(&pale_ink(0.035));
            }

            msg_label.setFrame(NSRect::new(
                NSPoint::new(bubble_x + pad_x, y + pad_y),
                NSSize::new(text_size.width, text_size.height),
            ));

            add_subview(&self.chat_ui.transcript_doc, &bubble_box);
            add_subview(&self.chat_ui.transcript_doc, &msg_label);
            y += bubble_h + 12.0;
        };

        for msg in &self.current_messages {
            let is_user = msg.role == "user";
            render_bubble(is_user, &msg.content);
        }
        if self.chat_streaming {
            let stream_text = if self.streaming_assistant_buffer.trim().is_empty() {
                "思考中… ▍".to_string()
            } else {
                format!("{} ▍", self.streaming_assistant_buffer)
            };
            render_bubble(false, &stream_text);
        }

        let doc_h = (y + 8.0).max(scroll_h);
        self.chat_ui.transcript_doc.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(scroll_w, doc_h),
        ));
        let bottom_y = (doc_h - scroll_h).max(0.0);
        self.chat_ui
            .transcript_doc
            .scrollPoint(NSPoint::new(0.0, bottom_y));
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
                Ok(OcrOutcome::Text(raw_text)) => {
                    let text = if self.settings.appearance().ocr_strip_newlines {
                        ttree_core::normalize_ocr_newlines(&raw_text)
                    } else {
                        raw_text
                    };
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
            UiEvent::HyMtProgress { progress } => {
                let text = progress.format_status();
                set_field_string(&self.settings_ui.hymt_info_label, &text);
                self.set_status(&text);
            }
            UiEvent::HyMtActionFinished { quant, result } => {
                self.hymt_downloading = false;
                self.refresh_hymt_ui_state();
                self.refresh_ai_header_badge();
                match result {
                    Ok(url) => {
                        self.set_status(&format!(
                            "✓ HY-MT1.5-1.8B ({}) 已启动并就绪 ({url})",
                            quant.code()
                        ));
                    }
                    Err(err) => {
                        self.set_status(&format!("⚠️ {err}"));
                    }
                }
            }
        }
    }

    pub fn poll_tick(&mut self) {
        let view = self.updater.view();
        if view.exit_for_install {
            ttree_ai::hy_mt_manager::stop_server();
            NSApplication::sharedApplication(self.mtm).terminate(None);
            return;
        }
        if view != self.update_view {
            self.settings_ui
                .update_button
                .setTitle(&NSString::from_str(view.button));
            self.settings_ui.update_button.setEnabled(!view.busy);
            self.settings_ui
                .update_status
                .setStringValue(&NSString::from_str(&view.status));
            self.update_view = view;
        }
        if self.window.isVisible() {
            match self.active_tab {
                TabMode::Translate => {
                    let current_src = get_text_view_string(&self.translate_ui.input_tv);
                    self.translate_ui
                        .input_placeholder
                        .setHidden(!current_src.is_empty());
                    if current_src != self.last_observed_source {
                        apply_text_view_line_spacing(&self.translate_ui.input_tv, 5.5);
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
                    self.chat_ui
                        .input_placeholder
                        .setHidden(!chat_in.is_empty());
                }
                TabMode::Settings => {}
            }
            self.sample_backdrop_and_step_ink(false);
        }

        if let Some(since) = self.pending_resize_save {
            if since.elapsed() >= RESIZE_SAVE_DEBOUNCE {
                self.pending_resize_save = None;
                self.settings.save_pane_layout(self.pane_layout);
            }
        }
    }

    pub fn on_window_did_move(&mut self) {
        self.sample_backdrop_and_step_ink(false);
    }

    pub fn sample_backdrop_and_step_ink(&mut self, immediate: bool) {
        if !self.window.isVisible() {
            return;
        }
        let now = Instant::now();
        let should_sample = immediate
            || self
                .last_backdrop_sample
                .is_none_or(|t| now.duration_since(t) >= BACKDROP_SAMPLE_INTERVAL);
        if should_sample {
            self.last_backdrop_sample = Some(now);
            let desired = if let Some(lum) = sample_window_backdrop_luminance(&self.window) {
                target_ink_tone_for_luminance(lum, self.target_ink_tone)
            } else {
                let app_ap = NSApplication::sharedApplication(self.mtm).effectiveAppearance();
                if is_dark_appearance(&app_ap) {
                    1.0
                } else {
                    0.0
                }
            };
            self.target_ink_tone = desired;
        }

        if immediate {
            let changed = (self.ink_tone - self.target_ink_tone).abs() > 1e-4
                || current_ink_tone().is_none_or(|c| (c - self.target_ink_tone).abs() > 1e-4);
            self.ink_tone = self.target_ink_tone;
            set_ink_tone(self.ink_tone);
            if changed {
                self.sync_adaptive_window_appearance();
                mark_view_tree_needs_display(&self.effect_view);
            }
            return;
        }

        let prev_dark = self.ink_tone >= 0.5;
        let (next_tone, changed) = step_ink_tone(self.ink_tone, self.target_ink_tone);
        if changed {
            self.ink_tone = next_tone;
            set_ink_tone(self.ink_tone);
            let next_dark = self.ink_tone >= 0.5;
            if prev_dark != next_dark {
                self.sync_adaptive_window_appearance();
            }
            mark_view_tree_needs_display(&self.effect_view);
        }
    }

    fn sync_adaptive_window_appearance(&self) {
        unsafe {
            let name = if self.ink_tone >= 0.5 {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            };
            if let Some(ap) = NSAppearance::appearanceNamed(name) {
                self.window.setAppearance(Some(&ap));
            }
        }
    }

    pub fn set_shortcut_recording(&mut self, recording: bool) {
        if self.recording_shortcut == recording {
            return;
        }
        self.recording_shortcut = recording;
        while global_hotkey::GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
        let desired = if recording {
            ParsedShortcuts::default()
        } else {
            self.parsed_shortcuts.clone()
        };
        match register_shortcuts(&self.hotkey_mgr, &mut self.registered_hotkeys, &desired) {
            Ok(()) if recording => self.set_status("请按组合键；Delete 清空，Esc 取消"),
            Ok(()) => self.set_status("录入完成，保存设置后生效"),
            Err(error) => self.set_status(&error),
        }
    }

    pub fn on_window_did_resign_key(&mut self) {
        self.set_shortcut_recording(false);
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
        let w = bounds.size.width.clamp(WINDOW_MIN_WIDTH, WINDOW_MAX_WIDTH);
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
                TabMode::Translate | TabMode::Chat => {
                    self.pane_layout.apply_window_resize(w, h, ai_enabled);
                }
                TabMode::Settings => {
                    self.pane_layout.width = w;
                }
            }
            self.pending_resize_save = Some(Instant::now());
        }

        self.relayout_all(w, h);
    }

    pub fn relayout_all(&self, w: f64, h: f64) {
        self.effect_view
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)));

        // 顶栏重排（距顶部边缘 10px–13px，距左右边缘 16px–18px）
        self.segmented_bg
            .setFrame(top_rect(h, 16.0, 10.0, 128.0, 28.0));
        let pill_x = if self.active_tab == TabMode::Chat {
            82.0
        } else {
            19.0
        };
        self.segmented_pill
            .setFrame(top_rect(h, pill_x, 12.0, 59.0, 24.0));
        self.tab_translate_btn
            .setFrame(top_rect(h, 19.0, 12.0, 59.0, 24.0));
        self.tab_chat_btn
            .setFrame(top_rect(h, 82.0, 12.0, 59.0, 24.0));
        self.settings_title_label
            .setFrame(top_rect(h, 18.0, 13.0, 120.0, 22.0));

        let pin_x = match self.active_tab {
            TabMode::Translate => w - 78.0,
            TabMode::Chat => w - 142.0,
            TabMode::Settings => w - 96.0,
        };
        self.pin_btn.setFrame(top_rect(h, pin_x, 10.0, 28.0, 28.0));
        self.new_chat_top_btn
            .setFrame(top_rect(h, w - 110.0, 10.0, 28.0, 28.0));
        self.history_top_btn
            .setFrame(top_rect(h, w - 78.0, 10.0, 28.0, 28.0));
        self.gear_btn
            .setFrame(top_rect(h, w - 44.0, 10.0, 28.0, 28.0));
        self.done_settings_btn
            .setFrame(top_rect(h, w - 60.0, 11.0, 44.0, 26.0));

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

        self.translate_ui.web_header_label.setFrame(top_rect(
            body_h,
            16.0,
            results_top + 7.0,
            240.0,
            15.0,
        ));
        self.translate_ui.web_copy_btn.setFrame(top_rect(
            body_h,
            w - 58.0,
            results_top + 4.0,
            46.0,
            20.0,
        ));
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

        self.translate_ui.ai_header_label.setFrame(top_rect(
            body_h,
            16.0,
            ai_top + 7.0,
            280.0,
            15.0,
        ));
        self.translate_ui.ai_copy_btn.setFrame(top_rect(
            body_h,
            w - 58.0,
            ai_top + 4.0,
            46.0,
            20.0,
        ));
        self.translate_ui.ai_scroll.setFrame(top_rect(
            body_h,
            2.0,
            ai_top + 22.0,
            w - 4.0,
            (ai_h - 26.0).max(40.0),
        ));

        // 对话视图重排
        self.relayout_chat_view(w, body_h);

        // 设置视图重排（全宽度自适应 + 按接入方式动态排布字段）
        self.relayout_settings_view(w, body_h);
    }

    fn relayout_chat_view(&self, w: f64, body_h: f64) {
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, body_h));
        self.chat_ui.root.setFrame(body_frame);
        let hist_h = 38.0;
        let hist_offset = if self.show_chat_history_bar {
            hist_h
        } else {
            0.0
        };
        self.chat_ui
            .history_bar
            .setFrame(top_rect(body_h, 0.0, 0.0, w, hist_h));
        self.chat_ui
            .history_surface
            .setFrame(top_rect(hist_h, 0.0, 0.0, w, hist_h));
        self.chat_ui
            .history_label
            .setFrame(top_rect(hist_h, 14.0, 10.0, 56.0, 18.0));
        self.chat_ui
            .conv_popup
            .setFrame(top_rect(hist_h, 72.0, 6.0, (w - 216.0).max(110.0), 25.0));
        self.chat_ui
            .new_conv_btn
            .setFrame(top_rect(hist_h, w - 136.0, 7.0, 64.0, 24.0));
        self.chat_ui
            .clear_hist_btn
            .setFrame(top_rect(hist_h, w - 66.0, 7.0, 54.0, 24.0));
        self.chat_ui
            .history_bottom_line
            .setFrame(top_rect(hist_h, 0.0, hist_h - 1.0, w, 1.0));

        let composer_h = self
            .pane_layout
            .clamped()
            .chat_input_height
            .min((body_h - hist_offset - 110.0).max(68.0));
        let transcript_h = (body_h - hist_offset - composer_h).max(110.0);
        self.chat_ui.transcript_scroll.setFrame(top_rect(
            body_h,
            0.0,
            hist_offset,
            w,
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
            6.0,
            composer_top + 6.0,
            w - 88.0,
            (composer_h - 12.0).max(40.0),
        ));
        self.chat_ui.input_placeholder.setFrame(top_rect(
            body_h,
            20.0,
            composer_top + 15.0,
            240.0,
            20.0,
        ));
        self.chat_ui.send_btn.setFrame(top_rect(
            body_h,
            w - 74.0,
            composer_top + (composer_h - 28.0) * 0.5,
            62.0,
            28.0,
        ));

        self.render_chat_transcript();
    }

    fn relayout_settings_view(&self, w: f64, body_h: f64) {
        let body_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, body_h));
        self.settings_ui.root.setFrame(body_frame);
        self.settings_ui
            .top_divider
            .setFrame(top_rect(body_h, 12.0, 0.0, w - 24.0, 1.0));
        let sidebar_w = 114.0;
        self.settings_ui.sidebar_divider.setFrame(top_rect(
            body_h,
            sidebar_w,
            1.0,
            1.0,
            body_h - 1.0,
        ));
        self.settings_ui.nav_translate_btn.setFrame(top_rect(
            body_h,
            10.0,
            14.0,
            sidebar_w - 20.0,
            32.0,
        ));
        self.settings_ui.nav_chat_btn.setFrame(top_rect(
            body_h,
            10.0,
            50.0,
            sidebar_w - 20.0,
            32.0,
        ));
        self.settings_ui.nav_shortcuts_btn.setFrame(top_rect(
            body_h,
            10.0,
            86.0,
            sidebar_w - 20.0,
            32.0,
        ));
        self.settings_ui.nav_general_btn.setFrame(top_rect(
            body_h,
            10.0,
            122.0,
            sidebar_w - 20.0,
            32.0,
        ));

        let content_x = sidebar_w + 18.0;
        let content_w = (w - content_x - 18.0).max(280.0);
        let sec_h = (body_h - 36.0).max(260.0);
        let sec_frame = top_rect(body_h, content_x, 8.0, content_w, sec_h);

        self.settings_ui.sec_translate.setFrame(sec_frame);
        self.settings_ui.sec_chat.setFrame(sec_frame);
        self.settings_ui.sec_shortcuts.setFrame(sec_frame);
        self.settings_ui.sec_general.setFrame(sec_frame);
        self.settings_ui.settings_status_label.setFrame(top_rect(
            body_h,
            content_x,
            body_h - 26.0,
            content_w,
            20.0,
        ));

        let lbl_w = 72.0;
        let field_x = 78.0;
        let field_w = (content_w - field_x).max(160.0);
        let row_step = 36.0;

        // ── 1. AI 翻译分区（按所选接入方式动态排布，底部留出宽敞多行提示词编辑框） ──
        let ai_backend = self.selected_ai_trans_backend();
        let ai_is_hymt = ai_backend == AiBackendKind::BuiltInHyMt;
        let ai_is_gguf = ai_backend == AiBackendKind::CandleGguf;
        let ai_is_http = !ai_is_hymt && !ai_is_gguf;
        let ai_needs_key = matches!(
            ai_backend,
            AiBackendKind::OpenAiCompatible | AiBackendKind::LocalServer
        );

        self.settings_ui.t_head.setFrame(top_rect(
            sec_h,
            0.0,
            4.0,
            (content_w - 116.0).max(160.0),
            20.0,
        ));
        self.settings_ui.ai_enabled_chk.setFrame(top_rect(
            sec_h,
            (content_w - 108.0).max(160.0),
            3.0,
            108.0,
            22.0,
        ));

        let mut y = 34.0;
        self.settings_ui
            .t_lbl_backend
            .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
        if !ai_is_http {
            self.settings_ui.btn_ai_def_url.setHidden(true);
            self.settings_ui
                .ai_backend_popup
                .setFrame(top_rect(sec_h, field_x, y, field_w, 26.0));
        } else {
            self.settings_ui.btn_ai_def_url.setHidden(false);
            self.settings_ui.ai_backend_popup.setFrame(top_rect(
                sec_h,
                field_x,
                y,
                (field_w - 74.0).max(120.0),
                26.0,
            ));
            self.settings_ui.btn_ai_def_url.setFrame(top_rect(
                sec_h,
                content_w - 68.0,
                y,
                68.0,
                25.0,
            ));
        }
        y += row_step;

        // 内置混元 HY-MT1.5 模式：与其它模式完全统一的左右两栏对齐（左 72px 标签 + 右 field_x..content_w 控件区）
        self.settings_ui.t_lbl_hymt_quant.setHidden(!ai_is_hymt);
        self.settings_ui.ai_hymt_quant_popup.setHidden(!ai_is_hymt);
        self.settings_ui.t_lbl_hymt_action.setHidden(!ai_is_hymt);
        self.settings_ui.btn_hymt_check.setHidden(!ai_is_hymt);
        self.settings_ui
            .btn_hymt_download_run
            .setHidden(!ai_is_hymt);
        self.settings_ui.btn_hymt_stop.setHidden(!ai_is_hymt);
        self.settings_ui.btn_hymt_delete.setHidden(!ai_is_hymt);
        self.settings_ui.t_lbl_hymt_status.setHidden(!ai_is_hymt);
        self.settings_ui.hymt_info_surface.setHidden(!ai_is_hymt);
        self.settings_ui.hymt_info_label.setHidden(!ai_is_hymt);

        if ai_is_hymt {
            self.settings_ui
                .t_lbl_hymt_quant
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            self.settings_ui
                .ai_hymt_quant_popup
                .setFrame(top_rect(sec_h, field_x, y, field_w, 26.0));
            y += row_step;

            self.settings_ui
                .t_lbl_hymt_action
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            let gap = 6.0;
            let w_check = 66.0;
            let w_stop = 44.0;
            let w_del = 44.0;
            let w_run = (field_w - w_check - w_stop - w_del - gap * 3.0).max(74.0);
            self.settings_ui
                .btn_hymt_check
                .setFrame(top_rect(sec_h, field_x, y, w_check, 25.0));
            self.settings_ui.btn_hymt_download_run.setFrame(top_rect(
                sec_h,
                field_x + w_check + gap,
                y,
                w_run,
                25.0,
            ));
            self.settings_ui.btn_hymt_stop.setFrame(top_rect(
                sec_h,
                content_w - w_del - gap - w_stop,
                y,
                w_stop,
                25.0,
            ));
            self.settings_ui.btn_hymt_delete.setFrame(top_rect(
                sec_h,
                content_w - w_del,
                y,
                w_del,
                25.0,
            ));
            y += row_step;

            self.settings_ui
                .t_lbl_hymt_status
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            let card_h = 42.0;
            self.settings_ui
                .hymt_info_surface
                .setFrame(top_rect(sec_h, field_x, y, field_w, card_h));
            set_wrapping_label_frame(
                &self.settings_ui.hymt_info_label,
                top_rect(
                    sec_h,
                    field_x + 8.0,
                    y + 5.0,
                    (field_w - 16.0).max(120.0),
                    card_h - 8.0,
                ),
            );
            y += card_h + 10.0;
        }

        // HTTP 模式：显示接口地址 + 模型名称；云端 API 或本地 MLX/LM Studio 显示 API 密钥（可选）
        self.settings_ui.t_lbl_url.setHidden(!ai_is_http);
        self.settings_ui.ai_base_url_field.setHidden(!ai_is_http);
        if ai_is_http {
            self.settings_ui
                .t_lbl_url
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            self.settings_ui
                .ai_base_url_field
                .setFrame(top_rect(sec_h, field_x, y, field_w, 25.0));
            y += row_step;
        }

        self.settings_ui.t_lbl_key.setHidden(!ai_needs_key);
        self.settings_ui.ai_key_field.setHidden(!ai_needs_key);
        self.settings_ui
            .ai_key_status_label
            .setHidden(!ai_needs_key);
        self.settings_ui.btn_ai_clr_key.setHidden(!ai_needs_key);
        if ai_needs_key {
            self.settings_ui
                .t_lbl_key
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            self.settings_ui.ai_key_field.setFrame(top_rect(
                sec_h,
                field_x,
                y,
                (field_w - 118.0).max(100.0),
                25.0,
            ));
            self.settings_ui.ai_key_status_label.setFrame(top_rect(
                sec_h,
                content_w - 114.0,
                y + 4.0,
                50.0,
                18.0,
            ));
            self.settings_ui.btn_ai_clr_key.setFrame(top_rect(
                sec_h,
                content_w - 58.0,
                y,
                58.0,
                25.0,
            ));
            y += row_step;
        }

        self.settings_ui.t_lbl_model.setHidden(!ai_is_http);
        self.settings_ui.ai_model_field.setHidden(!ai_is_http);
        self.settings_ui.btn_ai_discover.setHidden(!ai_is_http);
        self.settings_ui.ai_discovered_popup.setHidden(!ai_is_http);
        if ai_is_http {
            self.settings_ui
                .t_lbl_model
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            self.settings_ui.ai_model_field.setFrame(top_rect(
                sec_h,
                field_x,
                y,
                (field_w - 148.0).max(100.0),
                25.0,
            ));
            self.settings_ui.btn_ai_discover.setFrame(top_rect(
                sec_h,
                content_w - 142.0,
                y,
                68.0,
                25.0,
            ));
            self.settings_ui.ai_discovered_popup.setFrame(top_rect(
                sec_h,
                content_w - 70.0,
                y,
                70.0,
                25.0,
            ));
            y += row_step;
        }

        // GGUF 模式：仅在选择直接加载 .gguf 时显示模型路径与分词词表
        self.settings_ui.t_lbl_gguf.setHidden(!ai_is_gguf);
        self.settings_ui.ai_gguf_field.setHidden(!ai_is_gguf);
        self.settings_ui.btn_ai_gguf.setHidden(!ai_is_gguf);
        self.settings_ui.t_lbl_tok.setHidden(!ai_is_gguf);
        self.settings_ui.ai_tokenizer_field.setHidden(!ai_is_gguf);
        self.settings_ui.btn_ai_tok.setHidden(!ai_is_gguf);
        if ai_is_gguf {
            self.settings_ui
                .t_lbl_gguf
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            self.settings_ui.ai_gguf_field.setFrame(top_rect(
                sec_h,
                field_x,
                y,
                (field_w - 68.0).max(120.0),
                25.0,
            ));
            self.settings_ui
                .btn_ai_gguf
                .setFrame(top_rect(sec_h, content_w - 62.0, y, 62.0, 25.0));
            y += row_step;

            self.settings_ui
                .t_lbl_tok
                .setFrame(top_rect(sec_h, 0.0, y + 3.0, lbl_w, 18.0));
            self.settings_ui.ai_tokenizer_field.setFrame(top_rect(
                sec_h,
                field_x,
                y,
                (field_w - 68.0).max(120.0),
                25.0,
            ));
            self.settings_ui
                .btn_ai_tok
                .setFrame(top_rect(sec_h, content_w - 62.0, y, 62.0, 25.0));
            y += row_step;
        }

        // 多行翻译提示词大编辑框（上方标题 + 下方全宽多行滚动框）
        y += 2.0;
        self.settings_ui
            .t_lbl_prompt
            .setFrame(top_rect(sec_h, 0.0, y, content_w, 18.0));
        y += 20.0;
        let ai_prompt_h = (sec_h - y - 40.0).clamp(64.0, 220.0);
        set_multiline_input_frame(
            &self.settings_ui.ai_prompt_input,
            top_rect(sec_h, 0.0, y, content_w, ai_prompt_h),
        );
        set_multiline_input_hidden(&self.settings_ui.ai_prompt_input, false);
        y += ai_prompt_h + 8.0;
        self.settings_ui
            .save_ai_btn
            .setFrame(top_rect(sec_h, 0.0, y, 104.0, 30.0));

        // ── 2. AI 对话分区（按所选接入方式动态排布，底部留出宽敞多行系统提示词编辑框） ──
        let chat_backend = self.selected_chat_backend();
        let chat_is_hymt = chat_backend == AiBackendKind::BuiltInHyMt;
        let chat_is_gguf = chat_backend == AiBackendKind::CandleGguf;
        let chat_is_http = !chat_is_hymt && !chat_is_gguf;
        let chat_needs_key = matches!(
            chat_backend,
            AiBackendKind::OpenAiCompatible | AiBackendKind::LocalServer
        );

        self.settings_ui.c_head.setFrame(top_rect(
            sec_h,
            0.0,
            4.0,
            (content_w - 146.0).max(150.0),
            20.0,
        ));
        self.settings_ui.btn_sync_from_trans.setFrame(top_rect(
            sec_h,
            content_w - 138.0,
            2.0,
            138.0,
            25.0,
        ));

        let mut cy = 34.0;
        self.settings_ui
            .c_lbl_backend
            .setFrame(top_rect(sec_h, 0.0, cy + 3.0, lbl_w, 18.0));
        if !chat_is_http {
            self.settings_ui.btn_chat_def_url.setHidden(true);
            self.settings_ui
                .chat_backend_popup
                .setFrame(top_rect(sec_h, field_x, cy, field_w, 26.0));
        } else {
            self.settings_ui.btn_chat_def_url.setHidden(false);
            self.settings_ui.chat_backend_popup.setFrame(top_rect(
                sec_h,
                field_x,
                cy,
                (field_w - 74.0).max(120.0),
                26.0,
            ));
            self.settings_ui.btn_chat_def_url.setFrame(top_rect(
                sec_h,
                content_w - 68.0,
                cy,
                68.0,
                25.0,
            ));
        }
        cy += row_step;

        self.settings_ui.c_lbl_hymt_quant.setHidden(!chat_is_hymt);
        self.settings_ui
            .chat_hymt_quant_popup
            .setHidden(!chat_is_hymt);
        self.settings_ui.btn_chat_hymt_run.setHidden(!chat_is_hymt);
        if chat_is_hymt {
            self.settings_ui
                .c_lbl_hymt_quant
                .setFrame(top_rect(sec_h, 0.0, cy + 3.0, lbl_w, 18.0));
            self.settings_ui.chat_hymt_quant_popup.setFrame(top_rect(
                sec_h,
                field_x,
                cy,
                (field_w - 118.0).max(120.0),
                26.0,
            ));
            self.settings_ui.btn_chat_hymt_run.setFrame(top_rect(
                sec_h,
                content_w - 112.0,
                cy,
                112.0,
                26.0,
            ));
            cy += row_step;
        }

        self.settings_ui.c_lbl_url.setHidden(!chat_is_http);
        self.settings_ui
            .chat_base_url_field
            .setHidden(!chat_is_http);
        if chat_is_http {
            self.settings_ui
                .c_lbl_url
                .setFrame(top_rect(sec_h, 0.0, cy + 3.0, lbl_w, 18.0));
            self.settings_ui
                .chat_base_url_field
                .setFrame(top_rect(sec_h, field_x, cy, field_w, 25.0));
            cy += row_step;
        }

        self.settings_ui.c_lbl_key.setHidden(!chat_needs_key);
        self.settings_ui.chat_key_field.setHidden(!chat_needs_key);
        self.settings_ui
            .chat_key_status_label
            .setHidden(!chat_needs_key);
        self.settings_ui.btn_chat_clr_key.setHidden(!chat_needs_key);
        if chat_needs_key {
            self.settings_ui
                .c_lbl_key
                .setFrame(top_rect(sec_h, 0.0, cy + 3.0, lbl_w, 18.0));
            self.settings_ui.chat_key_field.setFrame(top_rect(
                sec_h,
                field_x,
                cy,
                (field_w - 118.0).max(100.0),
                25.0,
            ));
            self.settings_ui.chat_key_status_label.setFrame(top_rect(
                sec_h,
                content_w - 114.0,
                cy + 4.0,
                50.0,
                18.0,
            ));
            self.settings_ui.btn_chat_clr_key.setFrame(top_rect(
                sec_h,
                content_w - 58.0,
                cy,
                58.0,
                25.0,
            ));
            cy += row_step;
        }

        self.settings_ui.c_lbl_model.setHidden(!chat_is_http);
        self.settings_ui.chat_model_field.setHidden(!chat_is_http);
        self.settings_ui.btn_chat_discover.setHidden(!chat_is_http);
        self.settings_ui
            .chat_discovered_popup
            .setHidden(!chat_is_http);
        if chat_is_http {
            self.settings_ui
                .c_lbl_model
                .setFrame(top_rect(sec_h, 0.0, cy + 3.0, lbl_w, 18.0));
            self.settings_ui.chat_model_field.setFrame(top_rect(
                sec_h,
                field_x,
                cy,
                (field_w - 148.0).max(100.0),
                25.0,
            ));
            self.settings_ui.btn_chat_discover.setFrame(top_rect(
                sec_h,
                content_w - 142.0,
                cy,
                68.0,
                25.0,
            ));
            self.settings_ui.chat_discovered_popup.setFrame(top_rect(
                sec_h,
                content_w - 70.0,
                cy,
                70.0,
                25.0,
            ));
            cy += row_step;
        }

        self.settings_ui.c_lbl_gguf.setHidden(!chat_is_gguf);
        self.settings_ui.chat_gguf_field.setHidden(!chat_is_gguf);
        self.settings_ui.btn_chat_gguf.setHidden(!chat_is_gguf);
        if chat_is_gguf {
            self.settings_ui
                .c_lbl_gguf
                .setFrame(top_rect(sec_h, 0.0, cy + 3.0, lbl_w, 18.0));
            self.settings_ui.chat_gguf_field.setFrame(top_rect(
                sec_h,
                field_x,
                cy,
                (field_w - 68.0).max(120.0),
                25.0,
            ));
            self.settings_ui.btn_chat_gguf.setFrame(top_rect(
                sec_h,
                content_w - 62.0,
                cy,
                62.0,
                25.0,
            ));
            cy += row_step;
        }

        cy += 4.0;
        self.settings_ui
            .c_lbl_prompt
            .setFrame(top_rect(sec_h, 0.0, cy, content_w, 18.0));
        cy += 22.0;
        let chat_prompt_h = (sec_h - cy - 42.0).clamp(80.0, 220.0);
        set_multiline_input_frame(
            &self.settings_ui.chat_prompt_input,
            top_rect(sec_h, 0.0, cy, content_w, chat_prompt_h),
        );
        set_multiline_input_hidden(&self.settings_ui.chat_prompt_input, false);
        cy += chat_prompt_h + 10.0;
        self.settings_ui
            .save_chat_btn
            .setFrame(top_rect(sec_h, 0.0, cy, 104.0, 30.0));

        // ── 3. 快捷键分区（自适应宽度） ──
        let sc_lbl_w = 92.0;
        let sc_field_x = 96.0;
        let sc_field_w = (content_w - sc_field_x).max(160.0);
        self.settings_ui.sc_head.setFrame(top_rect(
            sec_h,
            0.0,
            4.0,
            (content_w - 104.0).max(140.0),
            20.0,
        ));
        self.settings_ui
            .btn_reset_sc
            .setFrame(top_rect(sec_h, content_w - 96.0, 2.0, 96.0, 25.0));

        let sc_rows = [
            (
                &self.settings_ui.sc_lbl_toggle,
                &self.settings_ui.sc_toggle_field,
            ),
            (&self.settings_ui.sc_lbl_ocr, &self.settings_ui.sc_ocr_field),
            (&self.settings_ui.sc_lbl_ai, &self.settings_ui.sc_ai_field),
            (
                &self.settings_ui.sc_lbl_sel_trans,
                &self.settings_ui.sc_sel_trans_field,
            ),
            (
                &self.settings_ui.sc_lbl_sel_ai,
                &self.settings_ui.sc_sel_ai_field,
            ),
        ];
        let mut sy = 40.0;
        for (lbl, fld) in sc_rows {
            lbl.setFrame(top_rect(sec_h, 0.0, sy + 3.0, sc_lbl_w, 18.0));
            fld.setFrame(top_rect(sec_h, sc_field_x, sy, sc_field_w, 25.0));
            sy += 38.0;
        }
        sy += 6.0;
        self.settings_ui
            .save_sc_btn
            .setFrame(top_rect(sec_h, 0.0, sy, 108.0, 30.0));
        sy += 42.0;
        set_wrapping_label_frame(
            &self.settings_ui.sc_hint,
            top_rect(sec_h, 0.0, sy, content_w, (sec_h - sy).max(56.0)),
        );

        // ── 4. 通用分区（自适应宽度） ──
        self.settings_ui
            .gen_head
            .setFrame(top_rect(sec_h, 0.0, 4.0, content_w, 20.0));
        self.settings_ui
            .gen_lbl_opacity
            .setFrame(top_rect(sec_h, 0.0, 40.0, 120.0, 18.0));
        self.settings_ui.opacity_val_label.setFrame(top_rect(
            sec_h,
            content_w - 60.0,
            40.0,
            60.0,
            18.0,
        ));
        self.settings_ui
            .opacity_slider
            .setFrame(top_rect(sec_h, 0.0, 64.0, content_w, 24.0));
        self.settings_ui
            .autostart_chk
            .setFrame(top_rect(sec_h, 0.0, 104.0, 220.0, 24.0));
        self.settings_ui
            .ocr_strip_newlines_chk
            .setFrame(top_rect(sec_h, 0.0, 134.0, 260.0, 24.0));
        self.settings_ui
            .ver_label
            .setFrame(top_rect(sec_h, 0.0, 174.0, content_w, 20.0));
        self.settings_ui
            .update_button
            .setFrame(top_rect(sec_h, 0.0, 204.0, 130.0, 30.0));
        set_wrapping_label_frame(
            &self.settings_ui.update_status,
            top_rect(sec_h, 0.0, 242.0, content_w, 58.0),
        );
        set_wrapping_label_frame(
            &self.settings_ui.gen_hint,
            top_rect(sec_h, 0.0, 308.0, content_w, (sec_h - 308.0).max(80.0)),
        );
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
            ActionTag::OcrStripNewlinesToggled,
            ActionTag::SettingsSecTranslate,
            ActionTag::SettingsSecChat,
            ActionTag::SettingsSecShortcuts,
            ActionTag::SettingsSecGeneral,
            ActionTag::UpdateApp,
            ActionTag::HyMtQuantChanged,
            ActionTag::HyMtCheckCompatibility,
            ActionTag::HyMtDownloadAndRun,
            ActionTag::HyMtStopServer,
            ActionTag::HyMtDeleteModel,
            ActionTag::ChatHyMtQuantChanged,
            ActionTag::ChatHyMtDownloadAndRun,
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

    #[test]
    fn cleans_markdown_markers_for_chat_bubbles() {
        let raw = "### 标题\n这是 **重点内容** 与 __下划线__。\n- 列表项";
        assert_eq!(
            clean_markdown_for_bubble(raw),
            "标题\n这是 重点内容 与 下划线。\n- 列表项"
        );
    }

    #[test]
    fn adaptive_ink_tone_hysteresis_and_smooth_transition_converges() {
        // 明亮背景 (lum = 0.92) -> 目标 0.0（深墨字）
        assert_eq!(target_ink_tone_for_luminance(0.92, 1.0), 0.0);
        // 暗色背景 (lum = 0.15) -> 目标 1.0（亮白字）
        assert_eq!(target_ink_tone_for_luminance(0.15, 0.0), 1.0);
        // 临界中间带 (0.44..0.56) 保持原状态防抖
        assert_eq!(target_ink_tone_for_luminance(0.50, 0.0), 0.0);
        assert_eq!(target_ink_tone_for_luminance(0.50, 1.0), 1.0);

        let mut tone = 1.0_f64;
        let mut steps = 0;
        while (tone - 0.0).abs() > 1e-6 {
            let (next, changed) = step_ink_tone(tone, 0.0);
            assert!(changed);
            assert!(next < tone);
            tone = next;
            steps += 1;
            assert!(steps <= 25);
        }
        assert_eq!(tone, 0.0);
        assert_eq!(step_ink_tone(0.0, 0.0), (0.0, false));
    }
}
