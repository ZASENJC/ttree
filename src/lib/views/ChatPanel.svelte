<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onDestroy, onMount, tick } from "svelte";
  import { getPinned, setPinned, type ChatMessage } from "../api/tauri";
  import EngineTabs, { type MainMode } from "../components/EngineTabs.svelte";
  import { chatStore } from "../stores/chat.svelte";
  import { isMacOS } from "../platform";

  interface Props {
    mode: MainMode;
    onModeChange: (mode: MainMode) => void;
    onSettings: () => void;
  }
  let { mode, onModeChange, onSettings }: Props = $props();

  const c = chatStore;
  const HISTORY_CONV_LIMIT = 100;

  type HistoryItem = ChatMessage & { index: number };
  let isPinned = $state(false);
  let showHistory = $state(false);
  let historyQuery = $state("");
  let messagesEl: HTMLElement | null = $state(null);
  let historyButton: HTMLButtonElement | null = $state(null);
  let historySearch: HTMLInputElement | null = $state(null);
  let historyPopover: HTMLElement | null = $state(null);
  let composerEl: HTMLTextAreaElement | null = $state(null);
  // 左侧悬浮呼出（hover 把手 → 浮层显示历史提问）
  let railOpen = $state(false);
  let closeRailTimer: ReturnType<typeof setTimeout> | null = null;
  let railPanelEl: HTMLElement | null = $state(null);

  // 左侧浮层用到的历史提问：略少、不参与搜索，避免与工具栏弹层争抢焦点。
  // 顺序为「旧 → 新」（与对话区消息时间方向一致）：先取最近 N 条，再反转。
  const RAIL_ITEM_LIMIT = 40;
  const RAIL_SCAN_LIMIT = 120;
  let railItems = $derived.by(() => {
    const userMsgs: HistoryItem[] = [];
    let scannedUserCount = 0;
    for (let index = c.state.messages.length - 1; index >= 0; index -= 1) {
      const message = c.state.messages[index];
      if (message.role !== "user") continue;
      scannedUserCount += 1;
      if (scannedUserCount > RAIL_SCAN_LIMIT) break;
      userMsgs.push({ ...message, index });
      if (userMsgs.length >= RAIL_ITEM_LIMIT) break;
    }
    // 复制后反转，避免就地修改累加器（与文件其余处的不可变风格一致）。
    return [...userMsgs].reverse();
  });

  // 顶栏「对话历史」：以对话为单位（每段一条，标题=首句提问），最新在最上。
  let historyConversations = $derived.by(() => {
    const q = historyQuery.trim().toLowerCase();
    const convs = [...c.state.conversations].reverse(); // 最新在前
    const filtered = q
      ? convs.filter((conv) => {
          if (conv.summary.toLowerCase().includes(q)) return true;
          return conv.messages.some((m) => m.content.toLowerCase().includes(q));
        })
      : convs;
    return filtered.slice(0, HISTORY_CONV_LIMIT);
  });

  onMount(() => {
    getPinned().then((pinned) => (isPinned = pinned));
  });

  // 组件销毁时清理悬浮关闭计时器，避免在已卸载组件上执行过期回调（与 Settings 一致）。
  onDestroy(() => {
    if (closeRailTimer) {
      clearTimeout(closeRailTimer);
      closeRailTimer = null;
    }
  });

  // 呼出后自动聚焦输入框（与翻译面板一致）
  $effect(() => {
    composerEl?.focus();
  });

  async function togglePinned() {
    isPinned = await setPinned(!isPinned);
  }

  async function openHistory() {
    // 每次打开回到「查看全部」状态
    historyQuery = "";
    showHistory = true;
    await tick();
    (historySearch ?? historyPopover)?.focus();
  }

  function closeHistory() {
    showHistory = false;
    historyQuery = "";
    queueMicrotask(() => historyButton?.focus());
  }

  function toggleHistory() {
    if (showHistory) {
      closeHistory();
      return;
    }

    void openHistory();
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (!showHistory || e.key !== "Escape") return;
    e.preventDefault();
    closeHistory();
  }

  function onHistoryKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeHistory();
      return;
    }

    if (e.key !== "Tab" || !historyPopover) return;

    const focusables = Array.from(
      historyPopover.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])',
      ),
    );
    if (focusables.length === 0) {
      e.preventDefault();
      historyPopover.focus();
      return;
    }

    const first = focusables[0];
    const last = focusables[focusables.length - 1];

    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
      return;
    }

    if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    const modifierPressed = isMacOS ? e.metaKey : e.ctrlKey;
    if (e.key === "Enter" && !e.shiftKey && !modifierPressed) {
      e.preventDefault();
      c.send();
    }
    if (e.key === "Escape") {
      e.preventDefault();
      // 弹层 / 左侧悬浮呼出打开时，Esc 先收起它们而不是隐藏窗口
      if (showHistory) {
        closeHistory();
      } else if (railOpen) {
        railOpen = false;
      } else {
        getCurrentWindow().hide();
      }
    }
  }

  // 搜索框内的 Esc：清空搜索词而非收起弹层；空搜索词时交给弹层关闭。
  function onSearchKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape" || !historyQuery) return;
    e.preventDefault();
    e.stopPropagation();
    historyQuery = "";
  }

  /** 滚动到指定消息并高亮闪烁。DOM 更新后执行，确保坐标正确。 */
  function scrollToMessage(index: number) {
    const node = messagesEl?.querySelector<HTMLElement>(`[data-index="${index}"]`);
    if (!node) return;
    node.scrollIntoView({ behavior: "smooth", block: "center" });
    node.classList.remove("flash");
    // 强制重启动画
    void node.offsetWidth;
    node.classList.add("flash");
  }

  function jumpToMessage(index: number) {
    // 左侧悬浮浮层（当前对话内跳转）使用
    queueMicrotask(() => scrollToMessage(index));
  }

  /** 打开一段历史对话：替换当前视图，并滚到最新一条消息。 */
  function openConversation(id: number) {
    closeHistory();
    c.openConversation(id);
    queueMicrotask(() => {
      messagesEl?.scrollTo({ top: messagesEl.scrollHeight, behavior: "auto" });
    });
  }

  function startNewConversation() {
    closeHistory();
    c.newConversation();
    queueMicrotask(() => {
      messagesEl?.scrollTo({ top: 0, behavior: "auto" });
      composerEl?.focus();
    });
  }

  // ── 左侧悬浮呼出 ──
  function openRail() {
    if (closeRailTimer) {
      clearTimeout(closeRailTimer);
      closeRailTimer = null;
    }
    railOpen = true;
  }

  function scheduleCloseRail() {
    if (closeRailTimer) clearTimeout(closeRailTimer);
    closeRailTimer = setTimeout(() => {
      railOpen = false;
      closeRailTimer = null;
    }, 160);
  }

  function jumpToMessageFromRail(index: number) {
    railOpen = false;
    queueMicrotask(() => scrollToMessage(index));
  }

  // 浮层展开后把列表滚到底部：顺序是「旧 → 新」，最新的提问默认可见。
  $effect(() => {
    if (!railOpen) return;
    void tick().then(() => {
      const list = railPanelEl?.querySelector<HTMLElement>(".rail-list");
      if (list) list.scrollTop = list.scrollHeight;
    });
  });

  function clearHistory() {
    c.clearHistory();
    closeHistory();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<main class="chat-panel">
  <header class="toolbar" data-tauri-drag-region>
    <EngineTabs {mode} onchange={onModeChange} />

    <div class="toolbar-actions">
      <button
        class="md-icon-button pin"
        class:active={isPinned}
        onclick={togglePinned}
        aria-pressed={isPinned}
        aria-label={isPinned ? "取消固定窗口" : "固定窗口"}
        title={isPinned ? "取消固定窗口" : "固定窗口"}
      >
        <span class="state-layer"></span>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M14 4.5c0-.83.67-1.5 1.5-1.5S17 3.67 17 4.5v6.15l2.27 2.27c.47.47.14 1.28-.53 1.28H13v5.55c0 .41-.34.75-.75.75s-.75-.34-.75-.75V14.2H5.76c-.67 0-1-.81-.53-1.28L7.5 10.65V4.5C7.5 3.67 8.17 3 9 3s1.5.67 1.5 1.5v5.55c0 .2-.08.39-.22.53L8.36 12.5h7.28l-1.92-1.92a.75.75 0 0 1-.22-.53V4.5Z" />
        </svg>
      </button>

      <button
        class="md-icon-button new-chat"
        onclick={startNewConversation}
        disabled={c.state.loading}
        aria-label="新对话"
        title="新对话"
      >
        <span class="state-layer"></span>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M18.25 3a.75.75 0 0 1 .75.75V8h4.25a.75.75 0 0 1 0 1.5H19v4.25a.75.75 0 0 1-1.5 0V9.5h-4.25a.75.75 0 0 1 0-1.5H17.5V3.75a.75.75 0 0 1 .75-.75ZM5 6.5A2.5 2.5 0 0 1 7.5 4h4.75a.75.75 0 0 1 0 1.5H7.5a1 1 0 0 0-1 1V18a1 1 0 0 0 1 1h8.5a1 1 0 0 0 1-1v-3.25a.75.75 0 0 1 1.5 0V18a2.5 2.5 0 0 1-2.5 2.5h-8.5A2.5 2.5 0 0 1 5 18V6.5Z" />
        </svg>
      </button>

      <button
        bind:this={historyButton}
        class="md-icon-button history"
        class:active={showHistory}
        onclick={toggleHistory}
        aria-expanded={showHistory}
        aria-haspopup="dialog"
        aria-controls="chat-history-popover"
        aria-label="查看对话历史"
        title="查看对话历史"
      >
        <span class="state-layer"></span>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M13 3a9 9 0 1 1-8.2 5.3.75.75 0 1 1 1.36.63A7.5 7.5 0 1 0 13 4.5c-2.2 0-4.17.95-5.54 2.46H10a.75.75 0 0 1 0 1.5H5.75A.75.75 0 0 1 5 7.71V3.5a.75.75 0 0 1 1.5 0v2.1A8.96 8.96 0 0 1 13 3Zm0 4.25c.41 0 .75.34.75.75v4.04l2.73 1.58a.75.75 0 1 1-.75 1.3l-3.1-1.8a.75.75 0 0 1-.38-.65V8c0-.41.34-.75.75-.75Z" />
        </svg>
      </button>

      <button class="md-icon-button gear" onclick={onSettings} aria-label="设置" title="设置">
        <span class="state-layer"></span>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M19.43 12.98c.04-.32.07-.65.07-.98s-.02-.66-.07-.98l2.11-1.65a.5.5 0 0 0 .12-.64l-2-3.46a.5.5 0 0 0-.6-.22l-2.49 1a7.3 7.3 0 0 0-1.7-.98L14.5 2.42A.5.5 0 0 0 14 2h-4a.5.5 0 0 0-.5.42L9.12 5.07c-.61.24-1.18.56-1.7.98l-2.49-1a.5.5 0 0 0-.6.22l-2 3.46a.5.5 0 0 0 .12.64l2.11 1.65c-.04.32-.06.65-.06.98s.02.66.07.98l-2.11 1.65a.5.5 0 0 0-.12.64l2 3.46c.13.22.39.31.6.22l2.49-1c.52.4 1.09.73 1.7.98l.38 2.65c.04.24.25.42.5.42h4c.25 0 .46-.18.5-.42l.38-2.65c.61-.24 1.18-.56 1.7-.98l2.49 1c.22.09.48 0 .6-.22l2-3.46a.5.5 0 0 0-.12-.64l-2.13-1.65ZM12 15.5A3.5 3.5 0 1 1 12 8a3.5 3.5 0 0 1 0 7.5Z" />
        </svg>
      </button>

      {#if showHistory}
        <!-- 点击外部区域（弹层以外的背景）自动收起 -->
        <button
          class="history-scrim"
          onclick={closeHistory}
          tabindex="-1"
          aria-label="关闭对话历史"
        ></button>
        <div
          bind:this={historyPopover}
          id="chat-history-popover"
          class="history-popover"
          role="dialog"
          aria-modal="true"
          aria-label="对话历史"
          tabindex="-1"
          onkeydown={onHistoryKeydown}
        >
          <div class="history-head">
            <span>对话历史</span>
            <div class="history-head-actions">
              <button onclick={startNewConversation} disabled={c.state.loading}>
                {c.state.loading ? "生成中" : "新对话"}
              </button>
              {#if c.state.conversations.length > 0}
                <button onclick={clearHistory} disabled={c.state.loading}>清空</button>
              {/if}
              <button onclick={closeHistory} aria-label="关闭对话历史">关闭</button>
            </div>
          </div>
          {#if c.state.conversations.length > 0}
            <div class="history-search">
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M9.5 3a6.5 6.5 0 1 0 4.03 11.6l4.43 4.44a.75.75 0 1 0 1.06-1.06l-4.43-4.44A6.5 6.5 0 0 0 9.5 3Zm-5 6.5a5 5 0 1 1 10 0 5 5 0 0 1-10 0Z" />
              </svg>
              <input
                bind:this={historySearch}
                type="search"
                bind:value={historyQuery}
                onkeydown={onSearchKeydown}
                placeholder="搜索对话"
                spellcheck="false"
                autocomplete="off"
                aria-label="搜索对话历史"
              />
            </div>
          {/if}
          {#if historyConversations.length === 0}
            <p class="history-empty">{historyQuery ? "没有找到相关对话" : "还没有对话记录"}</p>
          {:else}
            <div class="history-list">
              {#each historyConversations as conv (conv.id)}
                <button
                  class="history-item"
                  class:current={conv.id === c.state.currentId}
                  onclick={() => openConversation(conv.id)}
                >
                  <span class="history-item-meta">{conv.messages.length} 条</span>
                  <p>{conv.summary || "(空对话)"}</p>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </header>

  <!-- 左侧悬浮呼出：鼠标移到左缘把手即展开历史提问浮层 -->
  <div
    class="history-rail"
    class:open={railOpen}
    role="region"
    aria-label="历史提问预览"
    tabindex="-1"
    onpointerenter={openRail}
    onpointerleave={scheduleCloseRail}
    onfocusin={openRail}
    onfocusout={scheduleCloseRail}
  >
    <span class="rail-handle" aria-hidden="true">
      <svg viewBox="0 0 24 24"><path d="M9 6c.55 0 1 .45 1 1v10c0 .55-.45 1-1 1s-1-.45-1-1V7c0-.55.45-1 1-1Zm6 0c.55 0 1 .45 1 1v10c0 .55-.45 1-1 1s-1-.45-1-1V7c0-.55.45-1 1-1Z" /></svg>
    </span>

    {#if railOpen}
      <div class="rail-panel" bind:this={railPanelEl}>
        <div class="rail-head">
          <span>历史提问</span>
          <button onclick={() => openHistory()} aria-label="在工具栏打开完整历史">全部</button>
        </div>
        {#if railItems.length === 0}
          <p class="rail-empty">还没有提问</p>
        {:else}
          <div class="rail-list">
            {#each railItems as item (item.index)}
              <button class="rail-item" onclick={() => jumpToMessageFromRail(item.index)}>
                <p>{item.content}</p>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <section class="messages" bind:this={messagesEl} aria-label="AI 对话">
    {#if c.state.messages.length === 0}
      <p class="placeholder">和 AI 自由对话。</p>
    {/if}
    {#each c.state.messages as message, index (`${c.state.currentId}:${index}`)}
      <article
        class="message"
        class:user={message.role === "user"}
        class:assistant={message.role === "assistant"}
        data-index={index}
      >
        <span class="role">{message.role === "user" ? "你" : "AI"}</span>
        <p>{message.content}{#if c.state.loading && index === c.state.messages.length - 1}<span class="caret"></span>{/if}</p>
      </article>
    {/each}
    {#if c.state.error}
      <p class="error">{c.state.error}</p>
    {/if}
  </section>

  <footer class="composer">
    <textarea
      bind:this={composerEl}
      bind:value={c.state.input}
      onkeydown={onKeydown}
      placeholder="输入消息，回车发送"
      spellcheck="false"
      rows="1"
    ></textarea>
    <button onclick={c.send} disabled={c.state.loading || !c.state.input.trim()}>{c.state.loading ? "…" : "发送"}</button>
  </footer>
</main>

<style>
  .chat-panel {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    min-height: 0;
    overflow: hidden;
    border-radius: var(--radius-window);
    background: var(--panel-bg);
  }
  .toolbar {
    position: relative;
    z-index: 3;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
  }
  .toolbar-actions {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 0;
  }
  .md-icon-button {
    position: relative;
    display: inline-grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    overflow: hidden;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .md-icon-button svg {
    position: relative;
    z-index: 1;
    width: 18px;
    height: 18px;
    fill: currentColor;
  }
  .md-icon-button .state-layer {
    position: absolute;
    inset: 0;
    background: currentColor;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease);
  }
  .md-icon-button:hover {
    color: var(--text);
  }
  .md-icon-button:hover .state-layer {
    opacity: var(--state-hover);
  }
  .md-icon-button:active .state-layer {
    opacity: var(--state-pressed);
  }
  .md-icon-button.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .history-scrim {
    position: fixed;
    inset: 0;
    z-index: 19;
    padding: 0;
    border: none;
    background: transparent;
    cursor: default;
  }
  .history-popover {
    position: absolute;
    top: 52px;
    right: 36px;
    z-index: 20;
    width: min(330px, calc(100vw - 32px));
    max-height: 370px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface-overlay);
    box-shadow: var(--shadow-pop);
    transform-origin: top right;
    animation: md-pop-in var(--duration) var(--ease);
  }
  .history-head {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3);
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 700;
  }
  .history-head-actions {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .history-head button {
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: 600;
    padding: 4px 9px;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .history-head button:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .history-head button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .history-search {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-2) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface-high);
  }
  .history-search svg {
    flex: none;
    width: 16px;
    height: 16px;
    fill: var(--text-faint);
  }
  .history-search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-sm);
    outline: none;
  }
  /* 移除浏览器原生的搜索框清除按钮，统一用 Esc 清空 */
  .history-search input::-webkit-search-cancel-button {
    appearance: none;
  }
  .history-search input::placeholder {
    color: var(--text-faint);
  }
  .history-empty {
    margin: 0;
    padding: var(--space-4);
    color: var(--text-faint);
    font-size: var(--text-sm);
  }
  .history-list {
    max-height: 250px;
    overflow: auto;
    scrollbar-width: none;
  }
  .history-list::-webkit-scrollbar {
    display: none;
  }
  .history-item {
    width: calc(100% - var(--space-3));
    margin: 0 var(--space-2) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    padding: var(--space-3);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .history-item:hover {
    background: var(--surface-high);
  }
  .history-item.current {
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .history-item.current p {
    color: var(--accent-on-container);
    font-weight: 600;
  }
  .history-item-meta {
    display: block;
    margin-bottom: 3px;
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-weight: 700;
  }
  .history-item p {
    margin: 0;
    overflow: hidden;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    font-size: var(--text-sm);
    line-height: 1.45;
  }
  /* ── 左侧悬浮呼出 ── */
  .history-rail {
    position: absolute;
    top: 50%;
    left: 0;
    transform: translateY(-50%);
    z-index: 18;
    display: flex;
    align-items: stretch;
    /* 仅在悬浮区与浮层之间形成可命中的桥接 */
  }
  .rail-handle {
    display: inline-grid;
    place-items: center;
    width: 14px;
    align-self: stretch;
    margin: 80px 0;
    border-right: 1px solid transparent;
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    background: transparent;
    color: var(--text-faint);
    cursor: default;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease);
  }
  .rail-handle svg {
    width: 12px;
    height: 12px;
    fill: currentColor;
    opacity: 0.5;
    transition: opacity var(--duration-fast) var(--ease);
  }
  .history-rail:hover .rail-handle,
  .history-rail:focus-within .rail-handle {
    background: var(--accent-softer);
    color: var(--accent);
  }
  .history-rail:hover .rail-handle svg,
  .history-rail:focus-within .rail-handle svg {
    opacity: 1;
  }
  .rail-panel {
    margin-left: -2px;
    width: min(280px, 38vw);
    max-height: min(60vh, 460px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    border-left: none;
    border-radius: 0 var(--radius-lg) var(--radius-lg) 0;
    background: var(--surface-overlay);
    box-shadow: var(--shadow-pop);
    transform-origin: left center;
    animation: rail-slide-in var(--duration) var(--ease);
  }
  @keyframes rail-slide-in {
    from {
      opacity: 0;
      transform: translateX(-8px) scale(0.98);
    }
    to {
      opacity: 1;
      transform: translateX(0) scale(1);
    }
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .rail-head button {
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: 600;
    padding: 3px 8px;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .rail-head button:hover {
    background: var(--accent-soft);
  }
  .rail-empty {
    margin: 0;
    padding: var(--space-4);
    color: var(--text-faint);
    font-size: var(--text-sm);
  }
  .rail-list {
    flex: 1;
    min-height: 0;
    overflow: auto;
    scrollbar-width: none;
    padding: var(--space-2);
  }
  .rail-list::-webkit-scrollbar {
    display: none;
  }
  .rail-item {
    display: block;
    width: 100%;
    margin-bottom: var(--space-1);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    padding: var(--space-2) var(--space-3);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .rail-item:hover {
    background: var(--surface-high);
  }
  .rail-item p {
    margin: 0;
    overflow: hidden;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    font-size: var(--text-sm);
    line-height: 1.45;
  }
  .messages {
    position: relative;
    z-index: 1;
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-4);
    scrollbar-width: none;
  }
  .messages::-webkit-scrollbar {
    display: none;
  }
  .placeholder {
    margin: 0;
    color: var(--text-faint);
    font-size: var(--text-base);
  }
  .message {
    margin-bottom: var(--space-4);
  }
  .message.user {
    text-align: right;
  }
  .role {
    display: inline-block;
    margin-bottom: 5px;
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.03em;
  }
  .message p {
    display: inline-block;
    max-width: 82%;
    margin: 0;
    padding: 9px 13px;
    border-radius: var(--radius);
    background: var(--surface-high);
    color: var(--text);
    line-height: 1.62;
    text-align: left;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .message.user p {
    background: var(--accent-container);
    color: var(--accent-on-container);
  }
  .composer {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-4);
  }
  .composer::before {
    content: "";
    position: absolute;
    left: var(--space-4);
    right: var(--space-4);
    top: 0;
    height: 1px;
    background: var(--border);
  }
  .composer textarea {
    flex: 1;
    height: 44px;
    min-height: 44px;
    max-height: 44px;
    resize: none;
    overflow: auto;
    scrollbar-width: none;
    border: none;
    border-bottom: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-high);
    color: var(--text);
    font: inherit;
    line-height: 22px;
    padding: 10px 15px;
    outline: none;
    transition:
      border-color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .composer textarea:focus {
    outline: none;
  }
  .composer textarea::-webkit-scrollbar {
    display: none;
  }
  .composer button {
    align-self: flex-end;
    min-width: 64px;
    height: 44px;
    border: none;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
    cursor: pointer;
    transition:
      background var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease);
  }
  .composer button:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .composer button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .caret {
    display: inline-block;
    width: 2px;
    height: 1.1em;
    margin-left: 1px;
    vertical-align: text-bottom;
    background: var(--accent);
    border-radius: var(--radius-xs);
    animation: blink 1s steps(2, start) infinite;
  }
  @keyframes blink {
    50% { opacity: 0; }
  }
  :global(.message.flash) p {
    animation: msg-flash 1s var(--ease);
  }
  @keyframes msg-flash {
    0% {
      box-shadow: 0 0 0 3px var(--accent-soft);
    }
    100% {
      box-shadow: 0 0 0 0 transparent;
    }
  }
</style>
