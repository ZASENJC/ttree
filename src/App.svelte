<script lang="ts">
  import { onDestroy, onMount } from "svelte";
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
  import { appearance } from "./lib/stores/appearance.svelte";
  import { updater } from "./lib/stores/update.svelte";

  let mode = $state<MainMode>("translate");
  let view = $state<"main" | "settings">("main");

  onMount(() => {
    // 启动即加载持久化的对话历史与外观配置
    void chatStore.loadHistory();
    void appearance.load();

    // 启动时读取版本号，并在距上次检查满一周时静默检查更新
    void updater.loadVersion();
    void updater.autoCheckIfNeeded();

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
          // 划词内容：针对选中内容提问，沿用当前对话上下文
          void chatStore.sendText(text);
        } else {
          // 纯呼出：默认开一段新对话（历史提问仍保留在侧边悬浮浮层）
          chatStore.reset();
        }
      }),
    ];

    return () => {
      unlisteners.forEach((unlisten) => unlisten.then((un) => un()));
    };
  });

  onDestroy(() => updater.stopPeriodicCheck());
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
