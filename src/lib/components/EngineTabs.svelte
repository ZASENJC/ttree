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

<div class="segmented" role="tablist" aria-label="模式切换">
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
    display: inline-grid;
    grid-template-columns: repeat(2, 1fr);
    overflow: hidden;
    border: 1px solid var(--md-outline, var(--border));
    border-radius: 999px;
    background: var(--md-surface-container, var(--surface-sunken));
    box-shadow: inset 0 1px 0 oklch(100% 0 0 / 0.05);
  }
  .segment {
    position: relative;
    min-width: 72px;
    min-height: 40px;
    border: none;
    border-right: 1px solid var(--md-outline, var(--border));
    background: transparent;
    color: var(--md-on-surface-variant, var(--text-muted));
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.01em;
    padding: 0 16px;
    cursor: pointer;
    overflow: hidden;
    transition:
      color var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease);
  }
  .segment:last-child {
    border-right: none;
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
    opacity: 0.08;
  }
  .segment:active .state-layer {
    opacity: 0.12;
  }
  .segment.active {
    background: var(--md-secondary-container, var(--accent-soft));
    color: var(--md-on-secondary-container, var(--accent));
  }
</style>
