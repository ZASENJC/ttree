/** AI 对话状态与编排（Svelte 5 runes）。 */
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  appendChatHistory as apiAppendHistory,
  chat as apiChat,
  clearChatHistory as apiClearHistory,
  loadConversations as apiLoadConversations,
  onChatChunk,
  startNewConversation as apiStartNewConversation,
  type ChatMessage,
  type Conversation,
} from "../api/tauri";
import { recentChatContext } from "./chatContext";

interface ChatState {
  messages: ChatMessage[];
  input: string;
  loading: boolean;
  error: string | null;
  /** 全部历史对话（旧 → 新），供顶栏对话列表使用。 */
  conversations: Conversation[];
  /** 当前对话的 id；null 表示一段尚未落盘的新对话。 */
  currentId: number | null;
  /** 是否已从磁盘加载历史；避免空闪与重复覆盖。 */
  loaded: boolean;
}

function createChatStore() {
  const state = $state<ChatState>({
    messages: [],
    input: "",
    loading: false,
    error: null,
    conversations: [],
    currentId: null,
    loaded: false,
  });

  let unlisten: UnlistenFn | null = null;
  /** 标记用户已显式开新对话/切换对话：阻止 loadHistory 的自动恢复覆盖。 */
  let userStartedSession = false;
  /** 进行中的加载任务：避免并发调用重复请求并互相覆盖。 */
  let loadPromise: Promise<void> | null = null;

  /** 启动时加载全部历史对话，并默认显示最新的一段（恢复上次离开的位置）。 */
  function loadHistory(): Promise<void> {
    if (state.loaded || loadPromise) return loadPromise ?? Promise.resolve();
    loadPromise = doLoadHistory();
    return loadPromise;
  }

  async function doLoadHistory() {
    try {
      const conversations = await apiLoadConversations();
      state.conversations = conversations;
      // 仅在用户尚未介入（开新对话/切换对话）时，自动恢复最新一段。
      if (!userStartedSession) {
        const latest = conversations[conversations.length - 1];
        if (latest) {
          state.messages = [...latest.messages];
          state.currentId = latest.id;
        } else {
          state.currentId = null;
        }
      }
    } catch (e) {
      // 加载失败不阻塞对话，仅记录错误。
      state.error = typeof e === "string" ? e : String(e);
    } finally {
      state.loaded = true;
      loadPromise = null;
    }
  }

  async function ensureChunkListener() {
    if (unlisten) return;

    unlisten = await onChatChunk((chunk) => {
      const lastIndex = state.messages.length - 1;
      const last = state.messages[lastIndex];

      if (last && last.role === "assistant") {
        state.messages = state.messages.map((message, index) =>
          index === lastIndex
            ? { ...message, content: message.content + chunk.delta }
            : message,
        );
      }

      if (chunk.done) {
        state.loading = false;
        // 一轮结束：把这轮（用户消息 + 完整 AI 回复）持久化。
        persistLastRound();
      }
    });
  }

  /** 把最后一轮对话追加写入历史文件；AI 空回复不写入，避免污染历史。
   *  落盘后以后端为唯一事实来源重读对话列表，避免本地 id 猜测与后端序号错位。 */
  function persistLastRound() {
    const msgs = state.messages;
    if (msgs.length < 2) return;
    const user = msgs[msgs.length - 2];
    const assistant = msgs[msgs.length - 1];
    if (user.role !== "user" || assistant.role !== "assistant") return;
    if (assistant.content.trim() === "") return;

    const messages = msgs.slice();
    const summary = summarize(messages);

    void apiAppendHistory([
      { role: "user", content: user.content },
      { role: "assistant", content: assistant.content },
    ])
      .then(async () => {
        // 落盘成功：以后端为唯一事实来源重读，拿到权威 id 与列表，杜绝 id 错位。
        const conversations = await apiLoadConversations();
        state.conversations = conversations;
        // 新对话首次落盘：当前对话 id = 列表末尾（最新一段）。
        if (state.currentId === null) {
          const latest = conversations[conversations.length - 1];
          state.currentId = latest?.id ?? null;
        }
      })
      .catch((e) => {
        state.error = `保存对话历史失败: ${typeof e === "string" ? e : String(e)}`;
      });
  }

  /** 摘要：首句用户提问截断，与后端 truncate_summary 行为对齐。 */
  function summarize(messages: ChatMessage[]): string {
    const MAX = 40;
    const firstUser = messages.find((m) => m.role === "user");
    const firstLine = (firstUser?.content ?? "").split("\n")[0]?.trim() ?? "";
    if (firstLine.length <= MAX) return firstLine;
    return `${firstLine.slice(0, MAX)}…`;
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
      await apiChat(recentChatContext(state.messages.slice(0, -1)));
    } catch (e) {
      state.error = typeof e === "string" ? e : String(e);
      state.loading = false;
      // 失败/中断：移除空的 assistant 占位，不写入历史
      const last = state.messages[state.messages.length - 1];
      if (last && last.role === "assistant" && last.content === "") {
        state.messages = state.messages.slice(0, -1);
      }
    }
  }

  function clearHistory() {
    if (state.loading) {
      state.error = "AI 回复仍在生成，请等待完成后再清空历史。";
      return;
    }

    state.messages = [];
    state.conversations = [];
    state.currentId = null;
    state.error = null;
    void apiClearHistory().catch((e) => {
      state.error = `清空对话历史失败: ${typeof e === "string" ? e : String(e)}`;
    });
  }

  /** 开启一段新对话：写分隔标记 + 清空当前消息视图（历史保留在列表中）。 */
  function newConversation() {
    if (state.loading) {
      state.error = "AI 回复仍在生成，请等待完成后再开新对话。";
      return;
    }
    userStartedSession = true;
    state.messages = [];
    state.input = "";
    state.error = null;
    state.currentId = null;
    void apiStartNewConversation().catch((e) => {
      state.error = `开启新对话失败: ${typeof e === "string" ? e : String(e)}`;
    });
  }

  /** 打开指定历史对话，加载其全部消息到当前视图。 */
  function openConversation(id: number) {
    if (state.loading) {
      state.error = "AI 回复仍在生成，无法切换对话。";
      return;
    }
    const target = state.conversations.find((c) => c.id === id);
    if (!target) return;
    userStartedSession = true;
    state.messages = [...target.messages];
    state.currentId = id;
    state.input = "";
    state.error = null;
  }

  function reset() {
    // 呼出快捷键使用：开新对话，与 UI「新对话」按钮一致。
    newConversation();
  }

  return {
    get state() {
      return state;
    },
    loadHistory,
    send,
    sendText,
    clearHistory,
    newConversation,
    openConversation,
    reset,
  };
}

export const chatStore = createChatStore();
