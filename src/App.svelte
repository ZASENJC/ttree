<script lang="ts">
  import { onMount } from "svelte";
  import TranslatePanel from "./lib/components/TranslatePanel.svelte";
  import ChatPanel from "./lib/views/ChatPanel.svelte";
  import Settings from "./lib/views/Settings.svelte";
  import {
    onOpenSettings,
    onSelectedTranslate,
    onTriggerAiDialog,
    onTriggerOcr,
    onTriggerTranslate,
  } from "./lib/api/tauri";
  import type { MainMode } from "./lib/components/EngineTabs.svelte";
  import { chatStore } from "./lib/stores/chat.svelte";
  import { translation } from "./lib/stores/translation.svelte";

  let mode = $state<MainMode>("translate");
  let view = $state<"main" | "settings">("main");

  onMount(() => {
    const unlisteners = [
      onOpenSettings(() => (view = "settings")),
      onTriggerTranslate(() => {
        view = "main";
        mode = "translate";
      }),
      onTriggerOcr(() => {
        view = "main";
        mode = "translate";
        void translation.captureAndTranslate();
      }),
      onSelectedTranslate(({ text }) => {
        view = "main";
        mode = "translate";
        void translation.translateText(text);
      }),
      onTriggerAiDialog(({ text }) => {
        view = "main";
        mode = "chat";
        if (text) {
          void chatStore.sendText(text);
        }
      }),
    ];

    return () => {
      unlisteners.forEach((unlisten) => unlisten.then((un) => un()));
    };
  });
</script>

{#if view === "settings"}
  <Settings onClose={() => (view = "main")} />
{:else if mode === "chat"}
  <ChatPanel
    {mode}
    onModeChange={(nextMode) => (mode = nextMode)}
    onSettings={() => (view = "settings")}
  />
{:else}
  <TranslatePanel
    {mode}
    onModeChange={(nextMode) => (mode = nextMode)}
    onSettings={() => (view = "settings")}
  />
{/if}
