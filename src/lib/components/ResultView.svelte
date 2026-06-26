<script lang="ts">
  import {
    WEB_ENGINE_LABELS,
    WEB_ENGINES,
    type WebEngine,
  } from "../stores/translation.svelte";

  interface Props {
    webOutput: string;
    aiOutput: string;
    webEngine: WebEngine;
    webLoading: boolean;
    aiLoading: boolean;
    webError: string | null;
    aiError: string | null;
    error: string | null;
    onSelectWebEngine: (engine: WebEngine) => void;
  }
  let {
    webOutput,
    aiOutput,
    webEngine,
    webLoading,
    aiLoading,
    webError,
    aiError,
    error,
    onSelectWebEngine,
  }: Props = $props();

  let copied = $state<"web" | "ai" | null>(null);
  let engineMenuOpen = $state(false);

  async function copy(kind: "web" | "ai", text: string) {
    if (!text) return;
    await navigator.clipboard.writeText(text);
    copied = kind;
    setTimeout(() => (copied = null), 1400);
  }

  function selectEngine(engine: WebEngine) {
    engineMenuOpen = false;
    if (engine !== webEngine) {
      onSelectWebEngine(engine);
    }
  }
</script>

<svelte:window onclick={() => (engineMenuOpen = false)} />

<div class="result-grid">
  {#if error}
    <p class="error global-error">{error}</p>
  {/if}

  <section class="column" aria-label="网页翻译结果">
    <header class="column-head">
      <div class="engine-select">
        <button
          class="engine-trigger"
          onclick={(e) => {
            e.stopPropagation();
            engineMenuOpen = !engineMenuOpen;
          }}
          aria-haspopup="listbox"
          aria-expanded={engineMenuOpen}
          aria-label={`翻译引擎：${WEB_ENGINE_LABELS[webEngine]}，点击切换`}
        >
          {WEB_ENGINE_LABELS[webEngine]}
          <svg class="caret-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M7 10l5 5 5-5z" />
          </svg>
        </button>
        {#if engineMenuOpen}
          <ul class="engine-menu" role="listbox">
            {#each WEB_ENGINES as engine (engine)}
              <li>
                <button
                  role="option"
                  aria-selected={engine === webEngine}
                  class="engine-option"
                  class:active={engine === webEngine}
                  onclick={(e) => {
                    e.stopPropagation();
                    selectEngine(engine);
                  }}
                >
                  {WEB_ENGINE_LABELS[engine]}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      {#if webOutput}
        <button class="copy" onclick={() => copy("web", webOutput)} aria-label="复制翻译结果">
          {copied === "web" ? "已复制" : "复制"}
        </button>
      {/if}
    </header>

    {#if webError}
      <p class="error">{webError}</p>
    {:else if webOutput}
      <p class="output">{webOutput}</p>
    {:else if webLoading}
      <p class="placeholder">翻译中…</p>
    {:else}
      <p class="placeholder">译文显示在这里</p>
    {/if}
  </section>

  <section class="column ai-column" aria-label="AI 翻译结果">
    <header class="column-head">
      <span>AI</span>
      {#if aiOutput}
        <button class="copy" onclick={() => copy("ai", aiOutput)} aria-label="复制 AI 翻译结果">
          {copied === "ai" ? "已复制" : "复制"}
        </button>
      {/if}
    </header>

    {#if aiError}
      <p class="error">{aiError}</p>
    {:else if aiOutput}
      <p class="output">{aiOutput}{#if aiLoading}<span class="caret"></span>{/if}</p>
    {:else if aiLoading}
      <p class="placeholder">AI 翻译中…</p>
    {:else}
      <p class="placeholder">AI 译文显示在这里</p>
    {/if}
  </section>
</div>

<style>
  .result-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    min-height: 100%;
  }
  .global-error {
    grid-column: 1 / -1;
  }
  .column {
    position: relative;
    min-width: 0;
    min-height: 100%;
    padding: var(--space-3) var(--space-4) var(--space-2);
    overflow: hidden;
  }
  /* 两列之间只留一条细分隔线，不用卡片框 */
  .column + .column {
    box-shadow: inset 1px 0 0 var(--border);
  }
  .column-head {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .engine-select {
    position: relative;
  }
  .engine-trigger {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-height: 24px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-faint);
    font-family: inherit;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    padding: 3px 6px;
    margin-left: -6px;
    cursor: pointer;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .engine-trigger:hover,
  .engine-trigger[aria-expanded="true"] {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .caret-icon {
    width: 14px;
    height: 14px;
    fill: currentColor;
    transition: transform var(--duration-fast) var(--ease);
  }
  .engine-trigger[aria-expanded="true"] .caret-icon {
    transform: rotate(180deg);
  }
  .engine-menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 10;
    min-width: 132px;
    margin: 0;
    padding: 4px;
    list-style: none;
    border-radius: var(--radius);
    background: var(--surface-overlay);
    box-shadow: var(--shadow-pop);
    animation: md-pop-in var(--duration) var(--ease);
  }
  .engine-option {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-family: inherit;
    font-size: var(--text-sm);
    text-align: left;
    padding: 7px 10px;
    cursor: pointer;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .engine-option:hover {
    background: var(--surface-high);
  }
  .engine-option.active {
    color: var(--accent);
    font-weight: 600;
    background: var(--accent-soft);
  }
  .output {
    position: relative;
    z-index: 1;
    margin: 0;
    font-size: var(--text-lg);
    line-height: 1.62;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .placeholder {
    position: relative;
    z-index: 1;
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-faint);
  }
  .error {
    position: relative;
    z-index: 1;
    margin: 0;
    font-size: var(--text-sm);
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
    50% {
      opacity: 0;
    }
  }
  .copy {
    position: relative;
    z-index: 1;
    border: none;
    background: transparent;
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    padding: 3px 6px;
    margin-right: -6px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .copy:hover {
    color: var(--accent);
    background: var(--accent-soft);
  }

  @media (max-width: 560px) {
    .result-grid {
      grid-template-columns: 1fr;
    }
    .column + .column {
      box-shadow: inset 0 1px 0 var(--border);
    }
  }
</style>
