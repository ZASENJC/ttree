<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    getChatAiConfig,
    getOpenAiConfig,
    getPinned,
    getShortcutConfig,
    setChatAiConfig,
    setOpenAiConfig,
    setPinned,
    setShortcutConfig,
    setShortcutRecording,
    type ChatAiConfig,
    type OpenAiConfig,
    type ShortcutConfig,
  } from "../api/tauri";
  import {
    applyShortcutRecord,
    clearShortcutField,
    recordShortcutFromEvent,
    type ShortcutField,
  } from "../shortcuts/shortcutRecorder";
  import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let isPinned = $state(false);

  let cfg = $state<OpenAiConfig>({
    base_url: "https://api.openai.com/v1",
    api_key: "",
    model: "gpt-4o-mini",
    translate_prompt:
      "你是一个专业翻译引擎。将用户输入从 {source} 翻译成 {target}。只输出译文，不要解释、不要引号、不要附加任何说明。",
  });
  let chatCfg = $state<ChatAiConfig>({
    base_url: "https://api.openai.com/v1",
    api_key: "",
    model: "gpt-4o-mini",
    chat_prompt: "",
  });
  let shortcutCfg = $state<ShortcutConfig>({
    toggle: "CmdOrCtrl+Shift+Space",
    ocr: "CmdOrCtrl+Shift+S",
    ai_dialog: "",
    selection_translate: "",
    selection_ai_dialog: "",
  });
  type Section = "translate" | "chat" | "shortcuts" | "general";
  let activeSection = $state<Section>("translate");

  const sections: { id: Section; label: string }[] = [
    { id: "translate", label: "AI 翻译" },
    { id: "chat", label: "AI 对话" },
    { id: "shortcuts", label: "快捷键" },
    { id: "general", label: "通用" },
  ];

  let autostart = $state(false);
  let saved = $state(false);
  let chatSaved = $state(false);
  let shortcutSaved = $state(false);
  let shortcutError = $state<string | null>(null);
  let generalError = $state<string | null>(null);
  let recordingShortcut = $state<ShortcutField | null>(null);
  const shortcutButtons: Partial<Record<ShortcutField, HTMLButtonElement>> = {};

  const shortcutLabels: Record<ShortcutField, string> = {
    toggle: "呼出翻译",
    ocr: "截图 OCR",
    ai_dialog: "呼出 AI 对话",
    selection_translate: "划词翻译",
    selection_ai_dialog: "划词 AI 对话",
  };

  const shortcutFields: { field: ShortcutField; hint: string }[] = [
    { field: "toggle", hint: "呼出或收起翻译窗口" },
    { field: "ocr", hint: "框选屏幕区域并 OCR 翻译" },
    { field: "ai_dialog", hint: "呼出 AI 对话窗口" },
    { field: "selection_translate", hint: "捕获选中文本并直接翻译" },
    { field: "selection_ai_dialog", hint: "捕获选中文本并发送给 AI" },
  ];

  onMount(async () => {
    cfg = await getOpenAiConfig();
    chatCfg = await getChatAiConfig();
    shortcutCfg = await getShortcutConfig();
    getPinned().then((pinned) => (isPinned = pinned));
    try {
      autostart = await isEnabled();
    } catch {
      autostart = false;
    }
  });

  onDestroy(() => {
    if (recordingShortcut) {
      void setShortcutRecording(false);
    }
  });

  async function save() {
    await setOpenAiConfig($state.snapshot(cfg));
    cfg = await getOpenAiConfig();
    saved = true;
    setTimeout(() => (saved = false), 1400);
  }

  async function saveChat() {
    await setChatAiConfig($state.snapshot(chatCfg));
    chatCfg = await getChatAiConfig();
    chatSaved = true;
    setTimeout(() => (chatSaved = false), 1400);
  }

  async function startShortcutRecording(field: ShortcutField) {
    recordingShortcut = field;
    shortcutSaved = false;
    shortcutError = null;

    try {
      await setShortcutRecording(true);
    } catch {
      shortcutError = "启动快捷键录制失败，请重试。";
      recordingShortcut = null;
    }
  }

  async function stopShortcutRecording() {
    const field = recordingShortcut;
    recordingShortcut = null;

    try {
      await setShortcutRecording(false);
    } catch {
      shortcutError = "结束快捷键录制失败，请重新打开设置页。";
    }

    if (field) {
      shortcutButtons[field]?.focus();
    }
  }

  function handleShortcutKeydown(event: KeyboardEvent) {
    if (!recordingShortcut) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    const result = recordShortcutFromEvent(event);

    if (result.status === "cancelled") {
      stopShortcutRecording();
      shortcutError = null;
      return;
    }

    if (result.status === "pending") {
      return;
    }

    if (result.status === "invalid") {
      shortcutError = result.message;
      return;
    }

    shortcutCfg = applyShortcutRecord(shortcutCfg, recordingShortcut, result);
    shortcutError = null;
    stopShortcutRecording();
  }

  function clearShortcut(field: ShortcutField) {
    shortcutCfg = clearShortcutField(shortcutCfg, field);
    shortcutSaved = false;
    shortcutError = null;
    shortcutButtons[field]?.focus();
  }

  async function saveShortcuts() {
    shortcutError = null;

    if (recordingShortcut) {
      shortcutError = "请先完成当前快捷键录制，或按 Esc 取消。";
      return;
    }

    try {
      await setShortcutConfig($state.snapshot(shortcutCfg));
      shortcutSaved = true;
      setTimeout(() => (shortcutSaved = false), 1400);
    } catch (e) {
      shortcutError = typeof e === "string" ? e : String(e);
    }
  }

  async function togglePinned() {
    isPinned = await setPinned(!isPinned);
  }

  async function toggleAutostart() {
    const next = !autostart;
    generalError = null;
    try {
      if (next) {
        await enable();
      } else {
        await disable();
      }
      autostart = next;
    } catch {
      generalError = "切换开机自启失败，请稍后重试。";
    }
  }
</script>

<svelte:window onkeydown={handleShortcutKeydown} />

<section class="settings">
  <header class="toolbar" data-tauri-drag-region>
    <h2 class="title">设置</h2>
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
      <button class="close" onclick={onClose} aria-label="返回">完成</button>
    </div>
  </header>

  <div class="layout">
    <nav class="sidebar" aria-label="设置分类">
      {#each sections as sec}
        <button
          class="nav-item"
          class:active={activeSection === sec.id}
          onclick={() => (activeSection = sec.id)}
        >{sec.label}</button>
      {/each}
    </nav>

    <div class="panel">
      {#if activeSection === "translate"}
        <div class="group">
          <span class="group-title">AI 翻译配置（OpenAI 兼容）</span>
          <label class="field">
            <span>API 地址</span>
            <input type="text" bind:value={cfg.base_url} placeholder="https://api.openai.com/v1" spellcheck="false" />
          </label>
          <label class="field">
            <span>API Key</span>
            <input type="password" bind:value={cfg.api_key} placeholder="sk-…" spellcheck="false" />
          </label>
          <label class="field">
            <span>模型</span>
            <input type="text" bind:value={cfg.model} placeholder="gpt-4o-mini" spellcheck="false" />
          </label>
          <label class="field">
            <span>翻译提示词</span>
            <textarea bind:value={cfg.translate_prompt} placeholder="使用 &#123;source&#125; 和 &#123;target&#125; 作为语言占位符" spellcheck="false"></textarea>
          </label>
          <p class="hint">仅用于翻译页右侧 AI 译文；AI 对话页不会附加这个提示词。</p>
          <button class="save" onclick={save}>{saved ? "已保存" : "保存"}</button>
        </div>

      {:else if activeSection === "chat"}
        <div class="group">
          <span class="group-title">AI 对话配置（OpenAI 兼容）</span>
          <label class="field">
            <span>API 地址</span>
            <input type="text" bind:value={chatCfg.base_url} placeholder="https://api.openai.com/v1" spellcheck="false" />
          </label>
          <label class="field">
            <span>API Key</span>
            <input type="password" bind:value={chatCfg.api_key} placeholder="sk-…" spellcheck="false" />
          </label>
          <label class="field">
            <span>模型</span>
            <input type="text" bind:value={chatCfg.model} placeholder="gpt-4o-mini" spellcheck="false" />
          </label>
          <label class="field">
            <span>自定义提示词</span>
            <textarea bind:value={chatCfg.chat_prompt} placeholder="可选，作为 system 提示词附加在对话最前；留空则不附加" spellcheck="false"></textarea>
          </label>
          <p class="hint">仅用于顶部 AI 对话模式；不会使用翻译提示词。提示词留空即关闭。</p>
          <button class="save" onclick={saveChat}>{chatSaved ? "已保存" : "保存"}</button>
        </div>

      {:else if activeSection === "shortcuts"}
        <div class="group">
          <span class="group-title">快捷键</span>
          {#each shortcutFields as { field, hint } (field)}
            <div class="field">
              <span>{shortcutLabels[field]}</span>
              <div class="shortcut-row">
                <button
                  bind:this={shortcutButtons[field]}
                  type="button"
                  class="shortcut-recorder"
                  class:recording={recordingShortcut === field}
                  aria-label={recordingShortcut === field
                    ? "正在录制，按 Esc 取消"
                    : `当前为 ${shortcutCfg[field] || "未设置"}，点击录制`}
                  aria-pressed={recordingShortcut === field}
                  onclick={() => startShortcutRecording(field)}
                >
                  <span class="shortcut-value">
                    {recordingShortcut === field
                      ? "按下新的组合键…"
                      : shortcutCfg[field] || "未设置"}
                  </span>
                  <span class="shortcut-action">
                    {recordingShortcut === field ? "Esc 取消" : "点击录制"}
                  </span>
                </button>
                <button
                  type="button"
                  class="shortcut-clear"
                  disabled={!shortcutCfg[field] || recordingShortcut === field}
                  onclick={() => clearShortcut(field)}
                >
                  清空
                </button>
              </div>
              <p class="field-hint">{hint}</p>
            </div>
          {/each}
          <p class="shortcut-status" aria-live="polite">
            {recordingShortcut ? `正在录制${shortcutLabels[recordingShortcut]}快捷键，按下包含修饰键的组合键。` : "录制或清空后需点击“保存快捷键”才会生效。"}
          </p>
          {#if shortcutError}
            <p class="error" role="alert">{shortcutError}</p>
          {/if}
          <button class="save" onclick={saveShortcuts}>{shortcutSaved ? "已生效" : "保存快捷键"}</button>
          <p class="hint">每个快捷键都可单独设置或清空（留空即禁用），必须包含 Cmd/Ctrl/Option 修饰键。「呼出翻译」与「划词翻译」、「呼出 AI 对话」与「划词 AI 对话」可设为同一组合：有选中文本时执行划词，否则按呼出处理。示例：<kbd>CmdOrCtrl+Shift+Space</kbd>、<kbd>Option+Space</kbd>。</p>
        </div>

      {:else if activeSection === "general"}
        <div class="group">
          <span class="group-title">通用</span>
          <label class="toggle">
            <input type="checkbox" checked={autostart} onchange={toggleAutostart} />
            <span>开机自动启动</span>
          </label>
          <p class="hint">截图 OCR 需要"屏幕录制"权限。划词捕获优先使用"辅助功能"权限，未授权时会回退到临时复制并尽量恢复剪贴板。</p>
          {#if generalError}
            <p class="error" role="alert">{generalError}</p>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    height: 100vh;
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
    border-bottom: 1px solid var(--border);
  }
  .title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
  }
  .toolbar-actions {
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
  .md-icon-button.pin.active {
    background: var(--md-secondary-container, var(--accent-soft));
    color: var(--md-on-secondary-container, var(--accent));
    box-shadow: inset 0 0 0 1px oklch(70% 0.16 266 / 0.14);
  }
  .close {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: var(--text-sm);
    font-weight: 500;
    padding: 0 12px;
    cursor: pointer;
  }
  .layout {
    flex: 1;
    display: grid;
    grid-template-columns: 120px 1fr;
    min-height: 0;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    padding: var(--space-3) var(--space-2);
    border-right: 1px solid var(--border);
    gap: 2px;
  }
  .nav-item {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    text-align: left;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease), color var(--duration-fast) var(--ease);
  }
  .nav-item:hover {
    background: var(--surface-raised);
    color: var(--text);
  }
  .nav-item.active {
    background: var(--accent);
    color: var(--accent-text);
  }
  .panel {
    overflow-y: auto;
    padding: var(--space-4);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .group-title {
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-faint);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field span {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .field input,
  .field textarea {
    border: 1px solid var(--border);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--text-base);
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    outline: none;
    font-family: inherit;
    transition: border-color var(--duration-fast) var(--ease);
  }
  .field textarea {
    min-height: 96px;
    resize: vertical;
    line-height: 1.5;
  }
  .field input:focus,
  .field textarea:focus {
    border-color: var(--accent);
  }
  .shortcut-row {
    display: flex;
    align-items: stretch;
    gap: var(--space-2);
  }
  .shortcut-recorder {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    border: 1px solid var(--border);
    background: var(--surface-raised);
    color: var(--text);
    font-family: inherit;
    font-size: var(--text-base);
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    outline: none;
    cursor: pointer;
    text-align: left;
    transition:
      border-color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .shortcut-recorder:hover,
  .shortcut-recorder:focus-visible {
    border-color: var(--accent);
  }
  .shortcut-recorder.recording {
    background: var(--surface-sunken);
    border-color: var(--accent);
  }
  .shortcut-value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .shortcut-action {
    flex: none;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .shortcut-clear {
    flex: none;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    color: var(--text-muted);
    font-family: inherit;
    font-size: var(--text-sm);
    padding: 0 12px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition:
      border-color var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .shortcut-clear:hover:not(:disabled),
  .shortcut-clear:focus-visible {
    border-color: var(--accent);
    color: var(--accent);
  }
  .shortcut-clear:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .shortcut-status {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--text-muted);
  }
  .field-hint {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    line-height: 1.4;
    color: var(--text-faint);
  }
  .save {
    align-self: flex-start;
    margin-top: 4px;
    border: none;
    background: var(--accent);
    color: var(--accent-text);
    font-size: var(--text-sm);
    font-weight: 500;
    padding: 7px 18px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .save:hover {
    background: var(--accent-hover);
  }
  kbd {
    font-family: inherit;
    font-size: var(--text-xs);
    background: var(--surface-sunken);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px 8px;
    color: var(--text);
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
    cursor: pointer;
  }
  .hint {
    margin: 4px 0 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--text-faint);
  }
  .error {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--danger);
    background: var(--danger-soft);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
  }
</style>
