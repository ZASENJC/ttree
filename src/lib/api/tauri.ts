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

/** 普通 AI 对话：不附加翻译提示词，结果通过 chat-chunk 事件返回。 */
export function chat(messages: ChatMessage[]): Promise<void> {
  return invoke<void>("chat", { messages });
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
