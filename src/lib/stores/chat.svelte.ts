/** AI 对话状态与编排（Svelte 5 runes）。 */
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  chat as apiChat,
  onChatChunk,
  type ChatMessage,
} from "../api/tauri";

interface ChatState {
  messages: ChatMessage[];
  input: string;
  loading: boolean;
  error: string | null;
}

function createChatStore() {
  const state = $state<ChatState>({
    messages: [],
    input: "",
    loading: false,
    error: null,
  });

  let unlisten: UnlistenFn | null = null;

  async function ensureChunkListener() {
    if (unlisten) return;

    unlisten = await onChatChunk((chunk) => {
      const lastIndex = state.messages.length - 1;
      const last = state.messages[lastIndex];
      if (!last || last.role !== "assistant") return;

      state.messages = state.messages.map((message, index) =>
        index === lastIndex
          ? { ...message, content: message.content + chunk.delta }
          : message,
      );

      if (chunk.done) {
        state.loading = false;
      }
    });
  }

  async function send() {
    const text = state.input.trim();
    if (!text || state.loading) return;

    state.input = "";
    await sendMessage(text);
  }

  async function sendText(value: string) {
    const text = value.trim();
    if (!text) return;

    if (state.loading) {
      state.input = text;
      state.error = "上一条 AI 回复仍在生成，已将划词内容放入输入框。";
      return;
    }

    await sendMessage(text);
  }

  async function sendMessage(text: string) {
    state.error = null;
    state.messages = [
      ...state.messages,
      { role: "user", content: text },
      { role: "assistant", content: "" },
    ];
    state.loading = true;

    await ensureChunkListener();

    try {
      await apiChat(state.messages.slice(0, -1));
    } catch (e) {
      state.error = typeof e === "string" ? e : String(e);
      state.loading = false;
    }
  }

  function clearHistory() {
    state.messages = [];
    state.error = null;
  }

  function reset() {
    state.messages = [];
    state.input = "";
    state.loading = false;
    state.error = null;
  }

  return {
    get state() {
      return state;
    },
    send,
    sendText,
    clearHistory,
    reset,
  };
}

export const chatStore = createChatStore();
