import type { ChatMessage } from "../api/tauri";

export const CHAT_CONTEXT_LIMIT = 24;

export function recentChatContext(messages: ChatMessage[]): ChatMessage[] {
  const context = messages.slice(-CHAT_CONTEXT_LIMIT);

  // 丢弃切窗后开头所有「孤儿」assistant 消息：把 AI 回复作为上下文首条无意义，
  // 且会让上游认为对话由 assistant 开场。找到首条非 assistant，从其开始截取。
  const firstUser = context.findIndex((m) => m.role !== "assistant");
  if (firstUser <= 0) {
    // 0：首条已是非 assistant，原样返回；-1：全部是 assistant，丢弃为空数组。
    return firstUser === 0 ? context : [];
  }
  return context.slice(firstUser);
}
