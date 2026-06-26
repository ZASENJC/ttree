<script lang="ts">
  import { LANGUAGES, TARGET_LANGUAGES } from "../stores/languages";

  interface Props {
    source: string;
    target: string;
    onSource: (code: string) => void;
    onTarget: (code: string) => void;
  }
  let { source, target, onSource, onTarget }: Props = $props();
</script>

<div class="lang-row">
  <select
    class="lang"
    aria-label="源语言"
    value={source}
    onchange={(e) => onSource(e.currentTarget.value)}
  >
    {#each LANGUAGES as lang (lang.code)}
      <option value={lang.code}>{lang.name}</option>
    {/each}
  </select>

  <span class="arrow" aria-hidden="true">→</span>

  <select
    class="lang"
    aria-label="目标语言"
    value={target}
    onchange={(e) => onTarget(e.currentTarget.value)}
  >
    {#each TARGET_LANGUAGES as lang (lang.code)}
      <option value={lang.code}>{lang.name}</option>
    {/each}
  </select>
</div>

<style>
  .lang-row {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .lang {
    appearance: none;
    min-height: 28px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    padding: 4px 6px;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }
  .lang:hover {
    background: var(--accent-soft);
  }
  .lang:focus-visible {
    outline: none;
    background: var(--accent-soft);
  }
  .arrow {
    color: var(--text-faint);
    font-size: var(--text-sm);
    font-weight: 700;
  }
</style>
