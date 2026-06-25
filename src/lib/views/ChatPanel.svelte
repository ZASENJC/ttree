<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { getPinned, setPinned } from "../api/tauri";
  import EngineTabs, { type MainMode } from "../components/EngineTabs.svelte";
  import { chatStore } from "../stores/chat.svelte";

  interface Props {
    mode: MainMode;
    onModeChange: (mode: MainMode) => void;
    onSettings: () => void;
  }
  let { mode, onModeChange, onSettings }: Props = $props();

  const c = chatStore;
  let isPinned = $state(false);
  let showHistory = $state(false);

  let historyItems = $derived(
    c.state.messages
      .map((message, index) => ({ ...message, index }))
      .filter((message) => message.role === "user"),
  );

  onMount(() => {
    getPinned().then((pinned) => (isPinned = pinned));
  });

  async function togglePinned() {
    isPinned = await setPinned(!isPinned);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.metaKey) {
      e.preventDefault();
      c.send();
    }
    if (e.key === "Escape") {
      e.preventDefault();
      getCurrentWindow().hide();
    }
  }

  function clearHistory() {
    c.clearHistory();
    showHistory = false;
  }
</script>

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
        class="md-icon-button history"
        class:active={showHistory}
        onclick={() => (showHistory = !showHistory)}
        aria-expanded={showHistory}
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
        <div class="history-popover">
          <div class="history-head">
            <span>对话历史</span>
            {#if c.state.messages.length > 0}
              <button onclick={clearHistory}>清空</button>
            {/if}
          </div>
          {#if historyItems.length === 0}
            <p class="history-empty">暂无历史</p>
          {:else}
            <div class="history-list">
              {#each historyItems as item}
                <button class="history-item" onclick={() => (showHistory = false)}>
                  <span>你</span>
                  <p>{item.content}</p>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </header>

  <section class="messages" aria-label="AI 对话">
    {#if c.state.messages.length === 0}
      <p class="placeholder">和 AI 对话，不会使用翻译提示词。</p>
    {/if}
    {#each c.state.messages as message, index}
      <article class="message" class:user={message.role === "user"} class:assistant={message.role === "assistant"}>
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
      bind:value={c.state.input}
      onkeydown={onKeydown}
      placeholder="输入问题，回车发送，Shift+Enter 换行"
      spellcheck="false"
      rows="1"
    ></textarea>
    <button onclick={c.send} disabled={c.state.loading || !c.state.input.trim()}>{c.state.loading ? "…" : "发送"}</button>
  </footer>
</main>

<style>
  .chat-panel {
    display: flex;
    flex-direction: column;
    height: 100vh;
    min-height: 0;
    background: var(--surface);
    border-radius: var(--radius-window);
    overflow: hidden;
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4) var(--space-2);
  }
  .toolbar-actions {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px;
    border-radius: 999px;
    background: var(--md-surface-container, var(--surface-sunken));
  }
  .md-icon-button {
    position: relative;
    display: inline-grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--md-on-surface-variant, var(--text-muted));
    cursor: pointer;
    overflow: hidden;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease),
      transform var(--duration-fast) var(--ease);
  }
  .md-icon-button svg {
    position: relative;
    z-index: 1;
    width: 20px;
    height: 20px;
    fill: currentColor;
  }
  .md-icon-button .state-layer {
    position: absolute;
    inset: 0;
    background: currentColor;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease);
  }
  .md-icon-button:hover .state-layer {
    opacity: 0.08;
  }
  .md-icon-button:active {
    transform: scale(0.96);
  }
  .md-icon-button:active .state-layer {
    opacity: 0.12;
  }
  .md-icon-button.active {
    background: var(--md-secondary-container, var(--accent-soft));
    color: var(--md-on-secondary-container, var(--accent));
  }
  .history-popover {
    position: absolute;
    top: 48px;
    right: 36px;
    z-index: 20;
    width: min(320px, calc(100vw - 32px));
    max-height: 320px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface-raised);
    box-shadow: var(--shadow);
    transform-origin: top right;
    animation: popover-in var(--duration) var(--ease);
  }
  @keyframes popover-in {
    from {
      opacity: 0;
      transform: scale(0.86) translateY(-6px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }
  .history-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3);
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 700;
  }
  .history-head button {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: var(--text-xs);
    cursor: pointer;
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
    width: 100%;
    border: none;
    border-bottom: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    padding: var(--space-3);
    text-align: left;
    cursor: pointer;
  }
  .history-item:hover {
    background: var(--accent-soft);
  }
  .history-item span {
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
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    font-size: var(--text-sm);
    line-height: 1.4;
  }
  .messages {
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
    margin-bottom: 4px;
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-weight: 700;
  }
  .message p {
    display: inline-block;
    max-width: 82%;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface-sunken);
    color: var(--text);
    line-height: 1.6;
    text-align: left;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .message.user p {
    background: var(--accent-soft);
    color: var(--text);
  }
  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-4);
    border-top: 1px solid var(--border);
  }
  .composer textarea {
    flex: 1;
    height: 44px;
    min-height: 44px;
    max-height: 44px;
    resize: none;
    overflow: auto;
    scrollbar-width: none;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-raised);
    color: var(--text);
    font: inherit;
    line-height: 22px;
    padding: 10px 14px;
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
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-text);
    font-weight: 700;
    cursor: pointer;
  }
  .composer button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .error {
    margin: 0;
    color: var(--danger);
    background: var(--danger-soft);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
  }
  .caret {
    display: inline-block;
    width: 2px;
    height: 1.1em;
    margin-left: 1px;
    vertical-align: text-bottom;
    background: var(--accent);
    animation: blink 1s steps(2, start) infinite;
  }
  @keyframes blink {
    50% { opacity: 0; }
  }
</style>
