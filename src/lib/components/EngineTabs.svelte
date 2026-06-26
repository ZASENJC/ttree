<script lang="ts">
  export type MainMode = "translate" | "chat";

  interface Props {
    mode: MainMode;
    onchange: (mode: MainMode) => void;
  }
  let { mode, onchange }: Props = $props();

  const tabs: { id: MainMode; label: string }[] = [
    { id: "translate", label: "翻译" },
    { id: "chat", label: "AI" },
  ];
</script>

<div class="segmented" role="tablist" aria-label="模式切换" class:chat={mode === "chat"}>
  <span class="active-pill" aria-hidden="true"></span>
  {#each tabs as tab (tab.id)}
    <button
      role="tab"
      aria-selected={mode === tab.id}
      class="segment"
      class:active={mode === tab.id}
      onclick={() => onchange(tab.id)}
    >
      <span class="state-layer"></span>
      <span class="label">{tab.label}</span>
    </button>
  {/each}
</div>

<style>
  .segmented {
    position: relative;
    display: inline-grid;
    grid-template-columns: repeat(2, 1fr);
    min-height: 34px;
    padding: 3px;
    overflow: hidden;
    border-radius: var(--radius);
  }
  .active-pill {
    position: absolute;
    inset: 3px auto 3px 3px;
    width: calc(50% - 3px);
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    box-shadow: var(--pill-shadow);
    transition: transform var(--duration) var(--ease-emphasized);
  }
  .segmented.chat .active-pill {
    transform: translateX(100%);
  }
  .segment {
    position: relative;
    z-index: 1;
    min-width: 64px;
    min-height: 30px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.01em;
    padding: 0 14px;
    cursor: pointer;
    overflow: hidden;
    transition:
      color var(--duration-fast) var(--ease);
  }
  .state-layer {
    position: absolute;
    inset: 0;
    background: currentColor;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease);
  }
  .label {
    position: relative;
    z-index: 1;
  }
  .segment:hover .state-layer {
    opacity: var(--state-hover);
  }
  .segment:active .state-layer {
    opacity: var(--state-pressed);
  }
  .segment.active {
    color: var(--text);
  }
</style>
