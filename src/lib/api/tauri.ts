/** Tauri 后端调用与事件订阅封装。 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Engine = "openai" | "google" | "bing";

export interface TranslateRequest {
  text: string;
  engine: Engine;
  source: string;
  target: string;
}

export interface TranslateChunk {
  delta: string;
  done: boolean;
}

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

/** 单段对话：一组消息 + 摘要（首句用户提问，用作列表标题）。 */
export interface Conversation {
  /** 该对话在文件中的序号（0 = 最旧），唯一标识。 */
  id: number;
  messages: ChatMessage[];
  summary: string;
}

/** 普通 AI 对话：不附加翻译提示词，结果通过 chat-chunk 事件返回。 */
export function chat(messages: ChatMessage[]): Promise<void> {
  return invoke<void>("chat", { messages });
}

/** 读取全部对话历史（扁平消息流，向后兼容）。 */
export function loadChatHistory(): Promise<ChatMessage[]> {
  return invoke<ChatMessage[]>("load_chat_history");
}

/** 以对话为单位读取历史（按 session_start 标记切分）。 */
export function loadConversations(): Promise<Conversation[]> {
  return invoke<Conversation[]>("load_conversations");
}

/** 写入对话分隔标记，开启一段新对话（不删除已有历史）。 */
export function startNewConversation(): Promise<void> {
  return invoke<void>("start_new_conversation");
}

/** 追加一轮对话到历史文件（追加写，不重写）。 */
export function appendChatHistory(round: ChatMessage[]): Promise<void> {
  return invoke<void>("append_chat_history", { round });
}

/** 清空全部对话历史（删除历史文件）。 */
export function clearChatHistory(): Promise<void> {
  return invoke<void>("clear_chat_history");
}

/** 订阅 AI 对话 chunk 事件。 */
export function onChatChunk(
  handler: (chunk: TranslateChunk) => void,
): Promise<UnlistenFn> {
  return listen<TranslateChunk>("chat-chunk", (e) => handler(e.payload));
}

/**
 * 触发翻译。
 * - google：返回完整译文字符串
 * - openai：返回空串，结果通过 onChunk 流式到达
 */
export function translate(req: TranslateRequest): Promise<string> {
  return invoke<string>("translate", { req });
}

/** 订阅流式翻译 chunk 事件。 */
export function onChunk(
  handler: (chunk: TranslateChunk) => void,
): Promise<UnlistenFn> {
  return listen<TranslateChunk>("translate-chunk", (e) => handler(e.payload));
}

/** 触发交互式截图 + 系统 OCR，返回识别文本（用户取消时为空串）。 */
export function screenshotOcr(): Promise<string> {
  return invoke<string>("screenshot_ocr");
}

/** 截取全屏并打开区域选择 overlay 窗口，返回截图路径。 */
export function captureForSelection(): Promise<string> {
  return invoke<string>("capture_for_selection");
}

/** 对指定文件执行 OCR 并返回识别文本（自动删除文件）。 */
export function ocrFile(path: string): Promise<string> {
  return invoke<string>("ocr_file", { path });
}

/** 订阅选区完成事件（返回裁剪后的图片路径）。 */
export function onSelectionResult(
  handler: (path: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("selection-result", (e) => handler(e.payload));
}

/** 订阅选区取消事件。 */
export function onSelectionCancelled(
  handler: () => void,
): Promise<UnlistenFn> {
  return listen("selection-cancelled", () => handler());
}

/** 显示并聚焦主窗口。 */
export function showMain(): Promise<void> {
  return invoke<void>("show_main");
}

/** 读取固定窗口状态。 */
export function getPinned(): Promise<boolean> {
  return invoke<boolean>("get_pinned");
}

/** 设置固定窗口状态。 */
export function setPinned(pinned: boolean): Promise<boolean> {
  return invoke<boolean>("set_pinned", { pinned });
}

export interface OpenAiConfig {
  base_url: string;
  api_key: string;
  model: string;
  translate_prompt: string;
}

export interface ChatAiConfig {
  base_url: string;
  api_key: string;
  model: string;
  chat_prompt: string;
}

export interface ShortcutConfig {
  toggle: string;
  ocr: string;
  ai_dialog: string;
  selection_translate: string;
  selection_ai_dialog: string;
}

export interface AppearanceConfig {
  /** 面板整体透明度，0–1。后端会钳制到合法范围。 */
  panel_opacity: number;
}

export interface SelectedTranslatePayload {
  text: string;
}

export interface AiDialogPayload {
  text: string | null;
}

/** 读取 OpenAI 配置。 */
export function getOpenAiConfig(): Promise<OpenAiConfig> {
  return invoke<OpenAiConfig>("get_openai_config");
}

/** 保存 OpenAI 配置。 */
export function setOpenAiConfig(config: OpenAiConfig): Promise<void> {
  return invoke<void>("set_openai_config", { config });
}

/** 读取 AI 对话配置。 */
export function getChatAiConfig(): Promise<ChatAiConfig> {
  return invoke<ChatAiConfig>("get_chat_ai_config");
}

/** 保存 AI 对话配置。 */
export function setChatAiConfig(config: ChatAiConfig): Promise<void> {
  return invoke<void>("set_chat_ai_config", { config });
}

/** 读取快捷键配置。 */
export function getShortcutConfig(): Promise<ShortcutConfig> {
  return invoke<ShortcutConfig>("get_shortcut_config");
}

/** 保存快捷键配置并让后端立即重注册。 */
export function setShortcutConfig(config: ShortcutConfig): Promise<void> {
  return invoke<void>("set_shortcut_config", { config });
}

/** 录制快捷键时临时忽略全局快捷键触发。 */
export function setShortcutRecording(active: boolean): Promise<void> {
  return invoke<void>("set_shortcut_recording", { active });
}

/** 读取外观配置（面板透明度等）。 */
export function getAppearanceConfig(): Promise<AppearanceConfig> {
  return invoke<AppearanceConfig>("get_appearance_config");
}

/** 保存外观配置。 */
export function setAppearanceConfig(config: AppearanceConfig): Promise<void> {
  return invoke<void>("set_appearance_config", { config });
}

/** 订阅托盘"设置"菜单事件。 */
export function onOpenSettings(handler: () => void): Promise<UnlistenFn> {
  return listen("open-settings", () => handler());
}

/** 订阅截图 OCR 触发事件（快捷键按下时由后端发出）。 */
export function onTriggerOcr(handler: () => void): Promise<UnlistenFn> {
  return listen("trigger-ocr", () => handler());
}

/** 订阅呼出翻译页事件（toggle 快捷键显示窗口时由后端发出）。 */
export function onTriggerTranslate(handler: () => void): Promise<UnlistenFn> {
  return listen("trigger-translate", () => handler());
}

/** 订阅划词翻译事件。 */
export function onSelectedTranslate(
  handler: (payload: SelectedTranslatePayload) => void,
): Promise<UnlistenFn> {
  return listen<SelectedTranslatePayload>("trigger-selected-translate", (e) =>
    handler(e.payload),
  );
}

/** 订阅 AI 对话呼出事件。 */
export function onTriggerAiDialog(
  handler: (payload: AiDialogPayload) => void,
): Promise<UnlistenFn> {
  return listen<AiDialogPayload>("trigger-ai-dialog", (e) => handler(e.payload));
}
