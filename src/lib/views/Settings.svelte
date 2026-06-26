<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    getChatAiConfig,
    getOpenAiConfig,
    getPinned,
    getShortcutConfig,
    getAppearanceConfig,
    setChatAiConfig,
    setOpenAiConfig,
    setPinned,
    setShortcutConfig,
    setShortcutRecording,
    setAppearanceConfig,
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
  import { appearance } from "../stores/appearance.svelte";
  import { updater } from "../stores/update.svelte";
  import { isMacOS } from "../platform";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let isPinned = $state(false);
  /** 进入设置页前的固定状态快照；用于离开时恢复（仅当进入前未固定、由本页临时固定时回滚）。 */
  let pinnedBeforeEnter = false;
  /** 本页是否主动把窗口临时固定（区分「临时固定」与「用户手动固定」）。 */
  let pinnedBySettings = false;

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

  // 面板透明度：0–1，实时预览，停止拖动后落盘。
  let panelOpacity = $state(1);
  let opacitySaveTimer: ReturnType<typeof setTimeout> | null = null;

  const shortcutLabels: Record<ShortcutField, string> = {
    toggle: "呼出翻译",
    ocr: "截图 OCR",
    ai_dialog: "呼出 AI 对话",
    selection_translate: "划词翻译",
    selection_ai_dialog: "划词 AI 对话",
  };

  const shortcutFields: { field: ShortcutField; hint: string }[] = [
    { field: "toggle", hint: "显示或隐藏翻译窗口" },
    { field: "ocr", hint: "框选屏幕区域，识别文字并翻译" },
    { field: "ai_dialog", hint: "显示 AI 对话窗口" },
    { field: "selection_translate", hint: "翻译当前选中的文字" },
    { field: "selection_ai_dialog", hint: "把选中的文字发给 AI" },
  ];

  onMount(async () => {
    cfg = await getOpenAiConfig();
    chatCfg = await getChatAiConfig();
    shortcutCfg = await getShortcutConfig();
    // 进入设置页默认固定窗口：填表常需切到别处复制 API Key，固定后失焦不隐藏。
    // 仅当进入前未固定时临时固定；离开时恢复，避免改变用户原本的固定偏好。
    const pinned = await getPinned();
    isPinned = pinned;
    pinnedBeforeEnter = pinned;
    if (!pinned) {
      pinnedBySettings = true;
      isPinned = await setPinned(true);
    }
    try {
      autostart = await isEnabled();
    } catch {
      autostart = false;
    }
    try {
      const a = await getAppearanceConfig();
      panelOpacity = a.panel_opacity;
      appearance.set(panelOpacity);
    } catch {
      panelOpacity = 1;
    }
    void updater.loadVersion();
  });

  onDestroy(() => {
    if (recordingShortcut) {
      void setShortcutRecording(false).catch(() => {});
    }
    if (opacitySaveTimer) {
      clearTimeout(opacitySaveTimer);
    }
    // 离开设置页：若窗口是本页临时固定的（进入前未固定、用户也没手动改），
    // 恢复到进入前的未固定状态，避免改变用户原本的固定偏好。
    if (pinnedBySettings && !pinnedBeforeEnter) {
      void setPinned(false).catch(() => {});
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
    // 用户手动切换：清除「由本页临时固定」标记，离开时不再自动回滚。
    pinnedBySettings = false;
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

  // 拖动滑块时实时预览；停止拖动后（静默 400ms）再落盘，避免频繁写入。
  function onOpacityInput(event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    panelOpacity = value;
    appearance.set(value);
    if (opacitySaveTimer) {
      clearTimeout(opacitySaveTimer);
    }
    opacitySaveTimer = setTimeout(() => {
      void setAppearanceConfig({ panel_opacity: value }).catch(() => {});
    }, 400);
  }

  /** 点击「检查更新」：已有待安装版本则直接安装，否则发起检查并在命中时安装。 */
  async function onCheckUpdate() {
    const found = await updater.checkForUpdates();
    if (found) {
      await updater.downloadAndInstall();
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
          <span class="group-title">AI 翻译（OpenAI 兼容）</span>
          <label class="field">
            <span>接口地址</span>
            <input type="text" bind:value={cfg.base_url} placeholder="https://api.openai.com/v1" spellcheck="false" />
          </label>
          <label class="field">
            <span>API 密钥</span>
            <input type="password" bind:value={cfg.api_key} placeholder="sk-…" spellcheck="false" />
          </label>
          <label class="field">
            <span>模型</span>
            <input type="text" bind:value={cfg.model} placeholder="gpt-4o-mini" spellcheck="false" />
          </label>
          <label class="field">
            <span>翻译提示词</span>
            <textarea bind:value={cfg.translate_prompt} placeholder="用 &#123;source&#125; 和 &#123;target&#125; 表示源语言和目标语言" spellcheck="false"></textarea>
          </label>
          <p class="hint">这里只用于翻译页的 AI 译文，不影响 AI 对话。</p>
          <button class="save" onclick={save}>{saved ? "已保存" : "保存"}</button>
        </div>

      {:else if activeSection === "chat"}
        <div class="group">
          <span class="group-title">AI 对话（OpenAI 兼容）</span>
          <label class="field">
            <span>接口地址</span>
            <input type="text" bind:value={chatCfg.base_url} placeholder="https://api.openai.com/v1" spellcheck="false" />
          </label>
          <label class="field">
            <span>API 密钥</span>
            <input type="password" bind:value={chatCfg.api_key} placeholder="sk-…" spellcheck="false" />
          </label>
          <label class="field">
            <span>模型</span>
            <input type="text" bind:value={chatCfg.model} placeholder="gpt-4o-mini" spellcheck="false" />
          </label>
          <label class="field">
            <span>系统提示词</span>
            <textarea bind:value={chatCfg.chat_prompt} placeholder="可选，会作为系统提示加在对话开头。留空则不加" spellcheck="false"></textarea>
          </label>
          <p class="hint">只用于 AI 对话，不会套用翻译提示词。</p>
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
                    : `${shortcutLabels[field]}，当前 ${shortcutCfg[field] || "未设置"}，点击重新设置`}
                  aria-pressed={recordingShortcut === field}
                  onclick={() => startShortcutRecording(field)}
                >
                  <span class="shortcut-value">
                    {recordingShortcut === field
                      ? "按下新的快捷键…"
                      : shortcutCfg[field] || "未设置"}
                  </span>
                  <span class="shortcut-action">
                    {recordingShortcut === field ? "Esc 取消" : "设置"}
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
            {recordingShortcut ? `正在录制「${shortcutLabels[recordingShortcut]}」，请按下包含修饰键的组合键。` : "修改后点击“保存快捷键”生效。"}
          </p>
          {#if shortcutError}
            <p class="error" role="alert">{shortcutError}</p>
          {/if}
          <button class="save" onclick={saveShortcuts}>{shortcutSaved ? "已生效" : "保存快捷键"}</button>
          <p class="hint">每个快捷键都能单独设置或清空（留空即关闭）。必须包含 Cmd / Ctrl / Option 修饰键。「呼出翻译」和「划词翻译」、「呼出 AI 对话」和「划词 AI 对话」可以共用同一个键：有选中文本时走划词，没有则走呼出。例如 <kbd>CmdOrCtrl+Shift+Space</kbd>、<kbd>Option+Space</kbd>。</p>
        </div>

      {:else if activeSection === "general"}
        <div class="group">
          <span class="group-title">通用</span>

          <div class="opacity-field">
            <div class="opacity-head">
              <span>面板透明度</span>
              <span class="opacity-value">{Math.round(panelOpacity * 100)}%</span>
            </div>
            <input
              class="opacity-slider"
              type="range"
              min="0.35"
              max="1"
              step="0.01"
              value={panelOpacity}
              oninput={onOpacityInput}
              aria-label="面板透明度"
            />
          </div>

          <label class="toggle">
            <input type="checkbox" checked={autostart} onchange={toggleAutostart} />
            <span>开机自动启动</span>
          </label>

          <div class="update-field">
            <div class="update-head">
              <span>版本</span>
              <span class="update-version">{updater.currentVersion || "—"}</span>
            </div>
            {#if updater.releaseNotes}
              <p class="update-notes">{updater.releaseNotes}</p>
            {/if}
            {#if updater.status === "downloading" || updater.status === "installing"}
              <div class="update-progress">
                <div class="update-bar" style="width: {Math.round(updater.progress * 100)}%"></div>
              </div>
            {/if}
            <div class="update-actions">
              <button
                class="update-btn"
                onclick={onCheckUpdate}
                disabled={updater.status === "checking"
                  || updater.status === "downloading"
                  || updater.status === "installing"}
              >
                {#if updater.status === "checking"}
                  检查中…
                {:else if updater.status === "downloading"}
                  下载中 {Math.round(updater.progress * 100)}%
                {:else if updater.status === "installing"}
                  安装中…
                {:else if updater.status === "available"}
                  更新到 {updater.newVersion}
                {:else}
                  检查更新
                {/if}
              </button>
              {#if updater.message}
                <span
                  class="update-msg"
                  class:error={updater.status === "error"}
                  aria-live="polite"
                >{updater.message}</span>
              {/if}
            </div>
          </div>

          {#if isMacOS}
            <p class="hint">调整面板透明度可让底层桌面透出。截图 OCR 需要"屏幕录制"权限；划词翻译优先使用"辅助功能"权限，未授权时会临时用复制代替。</p>
          {:else}
            <p class="hint">调整面板透明度可让底层桌面透出。截图 OCR 和划词翻译在 Windows 上无需额外权限。</p>
          {/if}
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
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    border-radius: var(--radius-window);
    background: var(--panel-bg);
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
  .toolbar::after {
    content: "";
    position: absolute;
    left: var(--space-3);
    right: var(--space-3);
    bottom: 0;
    height: 1px;
    background: var(--border);
  }
  .title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    letter-spacing: -0.01em;
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
  .close {
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--accent);
    font-size: var(--text-sm);
    font-weight: 600;
    padding: 7px 12px;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .close:hover {
    background: var(--accent-soft);
  }
  .layout {
    position: relative;
    z-index: 1;
    flex: 1;
    display: grid;
    grid-template-columns: 140px 1fr;
    min-height: 0;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2);
    box-shadow: inset -1px 0 0 var(--border);
  }
  .nav-item {
    position: relative;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-muted);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    text-align: left;
    padding: 8px 12px;
    cursor: pointer;
    overflow: hidden;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease);
  }
  .nav-item:hover {
    background: var(--surface-high);
    color: var(--text);
  }
  .nav-item.active {
    background: var(--accent-container);
    color: var(--accent-on-container);
    font-weight: 600;
  }
  .panel {
    overflow-y: auto;
    padding: var(--space-4);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 520px;
  }
  .group-title {
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-faint);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .field span {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
  }
  .field input,
  .field textarea {
    border: 1px solid var(--border);
    background: var(--surface-high);
    color: var(--text);
    font-size: var(--text-base);
    padding: 9px 12px;
    border-radius: var(--radius-sm);
    outline: none;
    font-family: inherit;
    transition:
      border-color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .field textarea {
    min-height: 96px;
    resize: vertical;
    line-height: 1.5;
  }
  .field input:focus,
  .field textarea:focus {
    border-color: var(--accent);
    background: var(--surface-raised);
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
    background: var(--surface-high);
    color: var(--text);
    font-family: inherit;
    font-size: var(--text-base);
    padding: 9px 12px;
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
    background: var(--surface-raised);
  }
  .shortcut-recorder.recording {
    background: var(--accent-soft);
    border-color: var(--accent);
    animation: md-pulse 1.2s var(--ease) infinite;
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
    border: none;
    background: transparent;
    color: var(--accent);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    padding: 0 12px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .shortcut-clear:hover:not(:disabled),
  .shortcut-clear:focus-visible {
    background: var(--accent-soft);
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
    margin: 1px 0 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text-faint);
  }
  .save {
    align-self: flex-start;
    margin-top: 2px;
    border: none;
    background: var(--accent);
    color: var(--accent-text);
    font-size: var(--text-sm);
    font-weight: 600;
    padding: 8px 20px;
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
    background: var(--surface-high);
    border-radius: var(--radius-xs);
    padding: 2px 6px;
    color: var(--text);
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
    cursor: pointer;
  }
  .toggle input {
    accent-color: var(--accent);
  }
  .opacity-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .opacity-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
  }
  .opacity-value {
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .opacity-slider {
    width: 100%;
    height: 28px;
    margin: 0;
    cursor: pointer;
    accent-color: var(--accent);
  }
  .update-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .update-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-muted);
  }
  .update-version {
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .update-notes {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--text-muted);
    white-space: pre-wrap;
  }
  .update-progress {
    height: 4px;
    border-radius: 2px;
    background: var(--surface-high);
    overflow: hidden;
  }
  .update-bar {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width var(--duration) var(--ease);
  }
  .update-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .update-btn {
    border: none;
    background: var(--accent);
    color: var(--accent-text);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    padding: 8px 16px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .update-btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .update-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .update-msg {
    font-size: var(--text-xs);
    color: var(--text-faint);
  }
  .update-msg.error {
    color: var(--danger);
  }
  .hint {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--text-faint);
  }
  .error {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--danger);
  }
</style>
