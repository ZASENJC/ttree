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
    gap: var(--space-2);
  }
  .lang {
    appearance: none;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--text-sm);
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease);
  }
  .lang:hover {
    border-color: var(--border-strong);
  }
  .arrow {
    color: var(--text-faint);
    font-size: var(--text-sm);
  }
</style>
