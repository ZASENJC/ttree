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
          aria-label={`翻译源：${WEB_ENGINE_LABELS[webEngine]}，点击切换`}
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
      <p class="placeholder">译文将显示在这里</p>
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
      <p class="placeholder">AI 译文将显示在这里</p>
    {/if}
  </section>
</div>

<style>
  .result-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-4);
    min-height: 100%;
    padding: var(--space-3) 0;
  }
  .global-error {
    grid-column: 1 / -1;
  }
  .column {
    min-width: 0;
    padding-right: var(--space-3);
  }
  .ai-column {
    padding-left: var(--space-4);
    border-left: 1px solid var(--border);
  }
  .column-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .engine-select {
    position: relative;
  }
  .engine-trigger {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    border: none;
    background: transparent;
    color: var(--text-faint);
    font-family: inherit;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 2px 4px 2px 0;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: color var(--duration-fast) var(--ease);
  }
  .engine-trigger:hover {
    color: var(--accent);
  }
  .caret-icon {
    width: 14px;
    height: 14px;
    fill: currentColor;
  }
  .engine-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 10;
    margin: 0;
    padding: 4px;
    list-style: none;
    min-width: 120px;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px oklch(0% 0 0 / 0.18);
  }
  .engine-option {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: none;
    background: transparent;
    color: var(--text);
    font-family: inherit;
    font-size: var(--text-sm);
    text-align: left;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .engine-option:hover {
    background: var(--surface-sunken);
  }
  .engine-option.active {
    color: var(--accent);
    font-weight: 600;
  }
  .output {
    margin: 0;
    font-size: var(--text-lg);
    line-height: 1.6;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .placeholder {
    margin: 0;
    font-size: var(--text-base);
    color: var(--text-faint);
  }
  .error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--danger);
    padding: var(--space-2) var(--space-3);
    background: var(--danger-soft);
    border-radius: var(--radius-sm);
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
    50% {
      opacity: 0;
    }
  }
  .copy {
    border: none;
    background: var(--surface-sunken);
    color: var(--text-muted);
    font-size: var(--text-xs);
    padding: 3px 8px;
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
    .ai-column {
      padding-left: 0;
      padding-top: var(--space-3);
      border-left: none;
      border-top: 1px solid var(--border);
    }
  }
</style>
