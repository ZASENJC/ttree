<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { translation } from "../stores/translation.svelte";
  import { getPinned, setPinned } from "../api/tauri";
  import EngineTabs, { type MainMode } from "./EngineTabs.svelte";
  import LangSelector from "./LangSelector.svelte";
  import ResultView from "./ResultView.svelte";

  const MIN_SOURCE_HEIGHT = 88;
  const MIN_RESULT_HEIGHT = 120;
  const AUTO_TRANSLATE_DELAY_MS = 1000;

  const t = translation;
  let panel: HTMLElement | undefined = $state();
  let textarea: HTMLTextAreaElement | undefined = $state();
  let sourceHeight = $state(210);
  let isDraggingSplit = $state(false);
  let isPinned = $state(false);

  interface Props {
    mode: MainMode;
    onModeChange: (mode: MainMode) => void;
    onSettings: () => void;
  }
  let { mode, onModeChange, onSettings }: Props = $props();

  // 呼出后自动聚焦输入框
  $effect(() => {
    textarea?.focus();
  });

  // 输入有文字并静止 1 秒后自动翻译；继续输入会重置计时器。
  $effect(() => {
    const text = t.state.input.trim();
    if (!t.shouldAutoRun(text)) {
      return;
    }

    const timer = window.setTimeout(() => {
      if (t.shouldAutoRun(text) && t.state.input.trim() === text) {
        void t.run();
      }
    }, AUTO_TRANSLATE_DELAY_MS);

    return () => window.clearTimeout(timer);
  });

  // 初始化固定状态
  onMount(() => {
    getPinned().then((pinned) => (isPinned = pinned));
  });

  function onKeydown(e: KeyboardEvent) {
    // Enter 翻译；Cmd/Shift+Enter 换行
    if (e.key === "Enter" && !e.metaKey && !e.shiftKey) {
      e.preventDefault();
      t.run();
    }
    // Esc 隐藏窗口（用户显式关闭即使固定也允许）
    if (e.key === "Escape") {
      e.preventDefault();
      getCurrentWindow().hide();
    }
  }

  async function togglePinned() {
    isPinned = await setPinned(!isPinned);
  }

  function maxSourceHeight(): number {
    const height = panel?.clientHeight ?? 460;
    // 预留 toolbar、语言栏、间距和译文最小高度。
    return Math.max(MIN_SOURCE_HEIGHT, height - MIN_RESULT_HEIGHT - 128);
  }

  function clampSourceHeight(nextHeight: number): number {
    return Math.min(Math.max(nextHeight, MIN_SOURCE_HEIGHT), maxSourceHeight());
  }

  function startSplitResize(e: PointerEvent) {
    e.preventDefault();
    const startY = e.clientY;
    const startHeight = sourceHeight;
    isDraggingSplit = true;

    const onMove = (moveEvent: PointerEvent) => {
      sourceHeight = clampSourceHeight(startHeight + moveEvent.clientY - startY);
    };
    const onUp = () => {
      isDraggingSplit = false;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
  }
</script>

<main class="panel" bind:this={panel} class:dragging={isDraggingSplit}>
  <!-- 顶部工具栏：可拖拽，只放引擎与设置 -->
  <header class="toolbar" data-tauri-drag-region>
    <EngineTabs
      {mode}
      onchange={onModeChange}
    />
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
      <button class="md-icon-button gear" onclick={onSettings} aria-label="设置" title="设置">
        <span class="state-layer"></span>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M19.43 12.98c.04-.32.07-.65.07-.98s-.02-.66-.07-.98l2.11-1.65a.5.5 0 0 0 .12-.64l-2-3.46a.5.5 0 0 0-.6-.22l-2.49 1a7.3 7.3 0 0 0-1.7-.98L14.5 2.42A.5.5 0 0 0 14 2h-4a.5.5 0 0 0-.5.42L9.12 5.07c-.61.24-1.18.56-1.7.98l-2.49-1a.5.5 0 0 0-.6.22l-2 3.46a.5.5 0 0 0 .12.64l2.11 1.65c-.04.32-.06.65-.06.98s.02.66.07.98l-2.11 1.65a.5.5 0 0 0-.12.64l2 3.46c.13.22.39.31.6.22l2.49-1c.52.4 1.09.73 1.7.98l.38 2.65c.04.24.25.42.5.42h4c.25 0 .46-.18.5-.42l.38-2.65c.61-.24 1.18-.56 1.7-.98l2.49 1c.22.09.48 0 .6-.22l2-3.46a.5.5 0 0 0-.12-.64l-2.13-1.65ZM12 15.5A3.5 3.5 0 1 1 12 8a3.5 3.5 0 0 1 0 7.5Z" />
        </svg>
      </button>
    </div>
  </header>

  <!-- 原文区：拖动中间分割条调整高度 -->
  <section
    class="pane source-pane"
    aria-label="原文"
    style:height={`${sourceHeight}px`}
  >
    <textarea
      bind:this={textarea}
      bind:value={t.state.input}
      onkeydown={onKeydown}
      class="input"
      placeholder="输入文字，回车翻译"
      spellcheck="false"
    ></textarea>
  </section>

  <!-- 语言选择：置于上下两条线中间，左对齐；上方隐形命中区可拖动调高度 -->
  <div class="language-strip" class:active={isDraggingSplit}>
    <button
      class="resize-handle"
      aria-label="调整原文和译文高度"
      onpointerdown={startSplitResize}
    ></button>
    <LangSelector
      source={t.state.source}
      target={t.state.target}
      onSource={(c) => (t.state.source = c)}
      onTarget={(c) => (t.state.target = c)}
    />
  </div>

  <!-- 译文区：随原文高度变化自动获得剩余空间 -->
  <section class="pane result-pane" aria-label="译文">
    <ResultView
      webOutput={t.state.webOutput}
      aiOutput={t.state.aiOutput}
      webEngine={t.state.webEngine}
      webLoading={t.state.webLoading}
      aiLoading={t.state.aiLoading}
      webError={t.state.webError}
      aiError={t.state.aiError}
      error={t.state.error}
      onSelectWebEngine={(engine) => t.setWebEngine(engine)}
    />
  </section>
</main>

<style>
  .panel {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    min-height: 0;
    overflow: hidden;
    border-radius: var(--radius-window);
    background: var(--panel-bg);
    /* 阴影交给原生窗口 (tauri shadow:true)，避免双层圆角阴影叠加 */
  }
  .panel.dragging {
    user-select: none;
    cursor: ns-resize;
  }
  .toolbar {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
  }
  .pane {
    position: relative;
    z-index: 1;
    min-height: 0;
    overflow: auto;
    scrollbar-width: none;
  }
  .pane::-webkit-scrollbar {
    display: none;
  }
  .source-pane {
    flex: 0 0 auto;
    background: var(--surface-panel);
    box-shadow: inset 0 -1px 0 var(--border);
    transition: box-shadow var(--duration-fast) var(--ease);
  }
  .source-pane:focus-within {
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .result-pane {
    flex: 1 1 auto;
    min-height: 120px;
    /* 底部留出窗口圆角的空间，避免内容直角压在圆角窗角上 */
    padding-bottom: var(--space-2);
  }
  .input {
    width: 100%;
    height: 100%;
    min-height: 100%;
    resize: none;
    border: none;
    outline: none;
    overflow: auto;
    scrollbar-width: none;
    background: transparent;
    color: var(--text);
    font-size: var(--text-lg);
    line-height: 1.55;
    font-family: inherit;
    padding: var(--space-3) var(--space-4);
  }
  .input::-webkit-scrollbar {
    display: none;
  }
  .input::placeholder {
    color: var(--text-faint);
  }
  .language-strip {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    box-shadow: inset 0 -1px 0 var(--border);
  }
  .resize-handle {
    position: absolute;
    inset: -7px 0 auto 0;
    height: 14px;
    border: none;
    background: transparent;
    cursor: ns-resize;
  }
  .toolbar-actions {
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
  .md-icon-button.pin.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
</style>
