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

const SEND_BUSY_ERROR =
  "上一条消息正在准备或 AI 回复仍在生成，已将内容放入输入框。";
const SEND_PREPARING_ERROR = "消息正在准备发送，请稍候。";

interface ChatState {
  messages: ChatMessage[];
  input: string;
  loading: boolean;
  error: string | null;
  /** 全部历史对话（最久未活动 → 最近活动），供顶栏对话列表使用。 */
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
  /** 最近一次历史加载错误；成功重试时仅清除这条错误。 */
  let loadError: string | null = null;
  /** 新对话分隔标记的落盘任务；发送前必须等待它完成。 */
  let sessionStartPromise: Promise<boolean> | null = null;
  /** 新对话分隔标记失败后持续阻断发送，直到显式恢复到有效会话。 */
  let sessionStartError: string | null = null;
  /** 清空历史任务；发送/切换必须等待，避免与读取或追加并发。 */
  let clearPromise: Promise<void> | null = null;
  /** 发送正在等待历史加载或新会话标记；期间禁止改变会话边界。 */
  let sendPreparing = false;

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
      state.loaded = true;
      if (loadError && state.error === loadError) {
        state.error = null;
      }
      loadError = null;
    } catch (e) {
      state.loaded = false;
      loadError = typeof e === "string" ? e : String(e);
      state.error = loadError;
    } finally {
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
        // 一轮结束：把这轮（用户消息 + 完整 AI 回复）持久化。
        void persistLastRound().finally(() => {
          state.loading = false;
        });
      }
    });
  }

  /** 把最后一轮对话追加写入历史文件；AI 空回复不写入，避免污染历史。
   *  落盘后以后端为唯一事实来源重读对话列表，避免本地 id 猜测与后端序号错位。 */
  async function persistLastRound() {
    const msgs = state.messages;
    if (msgs.length < 2) return;
    const user = msgs[msgs.length - 2];
    const assistant = msgs[msgs.length - 1];
    if (user.role !== "user" || assistant.role !== "assistant") return;
    if (assistant.content.trim() === "") return;

    const conversationId = state.currentId;

    try {
      await apiAppendHistory(
        [
          { role: "user", content: user.content },
          { role: "assistant", content: assistant.content },
        ],
        conversationId,
      );
      const conversations = await apiLoadConversations();
      state.conversations = conversations;
      if (conversationId === null) {
        const latest = conversations[conversations.length - 1];
        state.currentId = latest?.id ?? null;
      }
    } catch (e) {
      state.error = `保存对话历史失败: ${typeof e === "string" ? e : String(e)}`;
    }
  }

  async function send() {
    const text = state.input.trim();
    if (!text) return;
    if (state.loading || sendPreparing) {
      preserveBusyInput(text);
      return;
    }

    state.input = "";
    await sendMessage(text);
  }

  async function sendText(value: string) {
    const text = value.trim();
    if (!text) return;

    if (state.loading || sendPreparing) {
      preserveBusyInput(text);
      return;
    }

    await sendMessage(text);
  }

  function preserveBusyInput(text: string) {
    state.input = text;
    state.error = SEND_BUSY_ERROR;
  }

  function preserveSessionBlockedInput(text: string) {
    state.input = text;
    state.error =
      sessionStartError ??
      "新对话尚未成功创建，请重新开启新对话、选择已有对话或清空历史后再发送。";
  }

  function clearSessionStartBlock() {
    sessionStartPromise = null;
    sessionStartError = null;
  }

  async function sendMessage(text: string) {
    if (state.loading || sendPreparing) {
      preserveBusyInput(text);
      return;
    }

    sendPreparing = true;
    try {
      if (clearPromise) {
        await clearPromise;
      }
      await loadHistory();
      if (!state.loaded) {
        state.input = text;
        return;
      }
      if (clearPromise) {
        await clearPromise;
      }
      if (state.loading) {
        preserveBusyInput(text);
        return;
      }

      if (sessionStartError) {
        preserveSessionBlockedInput(text);
        return;
      }
      if (sessionStartPromise) {
        const started = await sessionStartPromise;
        if (!started || sessionStartError) {
          preserveSessionBlockedInput(text);
          return;
        }
      }
      if (state.loading) {
        preserveBusyInput(text);
        return;
      }

      const keepBusyNotice =
        state.error === SEND_BUSY_ERROR &&
        state.input.trim() !== "" &&
        state.input.trim() !== text;
      if (!keepBusyNotice) {
        state.error = null;
      }
      state.messages = [
        ...state.messages,
        { role: "user", content: text },
        { role: "assistant", content: "" },
      ];
      state.loading = true;
    } finally {
      sendPreparing = false;
    }

    try {
      await ensureChunkListener();
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

  function clearHistory(): Promise<void> {
    if (clearPromise) return clearPromise;
    if (sendPreparing) {
      state.error = SEND_PREPARING_ERROR;
      return Promise.resolve();
    }
    if (state.loading) {
      state.error = "AI 回复仍在生成，请等待完成后再清空历史。";
      return Promise.resolve();
    }

    const clearing = clearHistoryAfterLoad().finally(() => {
      if (clearPromise === clearing) {
        clearPromise = null;
      }
    });
    clearPromise = clearing;
    return clearing;
  }

  async function clearHistoryAfterLoad() {
    await loadHistory();
    if (state.loading) {
      state.error = "AI 回复仍在生成，请等待完成后再清空历史。";
      return;
    }

    try {
      await apiClearHistory();
      userStartedSession = true;
      state.messages = [];
      state.conversations = [];
      state.currentId = null;
      state.loaded = true;
      loadError = null;
      clearSessionStartBlock();
      state.error = null;
    } catch (e) {
      state.error = `清空对话历史失败: ${typeof e === "string" ? e : String(e)}`;
    }
  }

  /** 开启一段新对话：写分隔标记 + 清空当前消息视图（历史保留在列表中）。 */
  function newConversation() {
    if (sendPreparing) {
      state.error = SEND_PREPARING_ERROR;
      return;
    }
    if (state.loading) {
      state.error = "AI 回复仍在生成，请等待完成后再开新对话。";
      return;
    }
    if (clearPromise) {
      state.error = "正在清空对话历史，请稍候。";
      return;
    }
    userStartedSession = true;
    state.messages = [];
    state.input = "";
    state.error = null;
    state.currentId = null;
    sessionStartError = null;
    const start = apiStartNewConversation()
      .then(() => {
        if (sessionStartPromise === start) {
          sessionStartError = null;
        }
        return true;
      })
      .catch((e) => {
        if (sessionStartPromise === start) {
          sessionStartError = `开启新对话失败: ${
            typeof e === "string" ? e : String(e)
          }。请重新开启新对话、选择已有对话或清空历史后再发送。`;
          state.error = sessionStartError;
        }
        return false;
      })
      .finally(() => {
        if (sessionStartPromise === start) {
          sessionStartPromise = null;
        }
      });
    sessionStartPromise = start;
  }

  /** 打开指定历史对话，加载其全部消息到当前视图。 */
  function openConversation(id: number) {
    if (sendPreparing) {
      state.error = SEND_PREPARING_ERROR;
      return;
    }
    if (state.loading) {
      state.error = "AI 回复仍在生成，无法切换对话。";
      return;
    }
    if (clearPromise) {
      state.error = "正在清空对话历史，请稍候。";
      return;
    }
    const target = state.conversations.find((c) => c.id === id);
    if (!target) return;
    userStartedSession = true;
    state.messages = [...target.messages];
    state.currentId = id;
    state.input = "";
    clearSessionStartBlock();
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
