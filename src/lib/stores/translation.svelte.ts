/** 翻译状态与编排（Svelte 5 runes）。 */
import {
  translate as apiTranslate,
  screenshotOcr,
  showMain,
  onChunk,
} from "../api/tauri";
import { ADAPTIVE_TARGET, resolveAdaptiveTarget } from "./languages";
import type { UnlistenFn } from "@tauri-apps/api/event";

/** 网页翻译引擎（可点按切换）。 */
export type WebEngine = "google" | "bing";

export const WEB_ENGINES: WebEngine[] = ["google", "bing"];

export const WEB_ENGINE_LABELS: Record<WebEngine, string> = {
  google: "谷歌翻译",
  bing: "必应翻译",
};

interface TranslationState {
  input: string;
  webOutput: string;
  aiOutput: string;
  webEngine: WebEngine;
  source: string;
  target: string;
  webLoading: boolean;
  aiLoading: boolean;
  webError: string | null;
  aiError: string | null;
  loading: boolean;
  error: string | null;
}

function createTranslationStore() {
  const state = $state<TranslationState>({
    input: "",
    webOutput: "",
    aiOutput: "",
    webEngine: "google",
    source: "auto",
    target: ADAPTIVE_TARGET,
    webLoading: false,
    aiLoading: false,
    webError: null,
    aiError: null,
    loading: false,
    error: null,
  });

  let unlisten: UnlistenFn | null = null;
  let lastRunText = "";

  function refreshLoading() {
    state.loading = state.webLoading || state.aiLoading;
  }

  async function ensureChunkListener() {
    if (unlisten) return;
    unlisten = await onChunk((chunk) => {
      state.aiOutput += chunk.delta;
      if (chunk.done) {
        state.aiLoading = false;
        refreshLoading();
      }
    });
  }

  async function run() {
    const text = state.input.trim();
    if (!text || state.loading) return;

    await runText(text);
  }

  /** 计算实际请求用的源/目标语言：自适应目标按输入在中/英间切换。 */
  function resolveLangPair(text: string): { source: string; target: string } {
    if (state.target === ADAPTIVE_TARGET) {
      return { source: "auto", target: resolveAdaptiveTarget(text) };
    }
    return { source: state.source, target: state.target };
  }

  async function runText(text: string) {
    lastRunText = text;
    state.error = null;
    state.webError = null;
    state.aiError = null;
    state.webOutput = "";
    state.aiOutput = "";
    state.webLoading = true;
    state.aiLoading = true;
    refreshLoading();

    await ensureChunkListener();

    const common = {
      text,
      ...resolveLangPair(text),
    };

    const webTask = apiTranslate({
      ...common,
      engine: state.webEngine,
    })
      .then((result) => {
        state.webOutput = result;
      })
      .catch((e) => {
        state.webError = typeof e === "string" ? e : String(e);
      })
      .finally(() => {
        state.webLoading = false;
        refreshLoading();
      });

    const aiTask = apiTranslate({
      ...common,
      engine: "openai",
    })
      .catch((e) => {
        state.aiError = typeof e === "string" ? e : String(e);
        state.aiLoading = false;
        refreshLoading();
      });

    await Promise.allSettled([webTask, aiTask]);
    refreshLoading();
  }

  /** 切换网页翻译引擎；若已有输入则只重跑网页侧翻译。 */
  async function setWebEngine(engine: WebEngine) {
    if (state.webEngine === engine) return;
    state.webEngine = engine;

    const text = state.input.trim();
    if (!text || state.webLoading) return;

    state.webError = null;
    state.webOutput = "";
    state.webLoading = true;
    refreshLoading();

    try {
      state.webOutput = await apiTranslate({
        text,
        ...resolveLangPair(text),
        engine: engine,
      });
    } catch (e) {
      state.webError = typeof e === "string" ? e : String(e);
    } finally {
      state.webLoading = false;
      refreshLoading();
    }
  }

  async function translateText(value: string) {
    const text = value.trim();
    if (!text || state.loading) return;
    state.input = text;
    await runText(text);
  }

  function setInput(v: string) {
    state.input = v;
  }

  function shouldAutoRun(text: string): boolean {
    return Boolean(text.trim()) && !state.loading && text.trim() !== lastRunText;
  }

  /** 截图 OCR：框选 → 识别 → 填入输入框 → 自动翻译。 */
  async function captureAndTranslate() {
    if (state.loading) return;
    state.error = null;
    try {
      const text = await screenshotOcr();
      if (!text.trim()) return; // 用户取消或未识别到文字
      await showMain(); // OCR 有结果后再唤起窗口
      await translateText(text);
    } catch (e) {
      state.error = typeof e === "string" ? e : String(e);
      await showMain();
    }
  }

  function reset() {
    state.input = "";
    state.webOutput = "";
    state.aiOutput = "";
    state.webError = null;
    state.aiError = null;
    state.error = null;
    state.webLoading = false;
    state.aiLoading = false;
    state.loading = false;
  }

  return {
    get state() {
      return state;
    },
    run,
    translateText,
    setInput,
    shouldAutoRun,
    setWebEngine,
    captureAndTranslate,
    reset,
  };
}

export const translation = createTranslationStore();
