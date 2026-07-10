/** 翻译状态与编排（Svelte 5 runes）。 */
import {
  translate as apiTranslate,
  screenshotOcr,
  showMain,
  onChunk,
  type TranslateChunk,
  type TranslateRequest,
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

const QUEUE_NOTICE = "上一条翻译仍在生成，已排队最新内容。";

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

export interface TranslationDependencies {
  translate: (req: TranslateRequest) => Promise<string>;
  screenshotOcr: () => Promise<string>;
  showMain: () => Promise<void>;
  onChunk: (
    handler: (chunk: TranslateChunk) => void,
  ) => Promise<UnlistenFn>;
}

const defaultDependencies: TranslationDependencies = {
  translate: apiTranslate,
  screenshotOcr,
  showMain,
  onChunk,
};

export function createTranslationStore(
  deps: TranslationDependencies = defaultDependencies,
) {
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
  let listenerPromise: Promise<void> | null = null;
  let lastRunText = "";
  let activeText = "";
  let queuedText: string | null = null;
  let drainingQueue = false;
  let drainGeneration = 0;
  let runGeneration = 0;
  let webRequestId = 0;
  let aiRequestId = 0;

  function refreshLoading() {
    state.loading = state.webLoading || state.aiLoading;
  }

  async function ensureChunkListener() {
    if (unlisten) return;
    if (!listenerPromise) {
      listenerPromise = deps
        .onChunk((chunk) => {
          if (!state.aiLoading || chunk.request_id !== aiRequestId) return;
          state.aiOutput += chunk.delta;
          if (chunk.done) {
            state.aiLoading = false;
            refreshLoading();
          }
        })
        .then((stop) => {
          unlisten = stop;
        })
        .finally(() => {
          listenerPromise = null;
        });
    }
    await listenerPromise;
  }

  async function run() {
    const text = state.input.trim();
    if (!text) return;
    if (state.loading) {
      if (text !== activeText) {
        queueText(text);
      }
      return;
    }

    await runText(text);
  }

  /** 计算实际请求用的源/目标语言：自适应目标按输入在中/英间切换。 */
  function resolveLangPair(text: string): { source: string; target: string } {
    if (state.target === ADAPTIVE_TARGET) {
      return { source: "auto", target: resolveAdaptiveTarget(text) };
    }
    return { source: state.source, target: state.target };
  }

  function toError(e: unknown): string {
    return typeof e === "string" ? e : String(e);
  }

  function queueText(text: string) {
    queuedText = text;
    state.input = text;
    state.error = QUEUE_NOTICE;
  }

  function clearQueuedText() {
    queuedText = null;
    if (state.error === QUEUE_NOTICE) state.error = null;
  }

  async function runWeb(
    text: string,
    engine: WebEngine,
    langPair: { source: string; target: string },
  ) {
    const requestId = ++webRequestId;
    state.webError = null;
    state.webOutput = "";
    state.webLoading = true;
    refreshLoading();

    try {
      const result = await deps.translate({
        request_id: requestId,
        text,
        ...langPair,
        engine,
      });
      if (requestId === webRequestId) {
        state.webOutput = result;
      }
    } catch (e) {
      if (requestId === webRequestId) {
        state.webError = toError(e);
      }
    } finally {
      if (requestId === webRequestId) {
        state.webLoading = false;
        refreshLoading();
      }
    }
  }

  async function runAi(
    text: string,
    langPair: { source: string; target: string },
  ) {
    const requestId = ++aiRequestId;
    state.aiError = null;
    state.aiOutput = "";
    state.aiLoading = true;
    refreshLoading();

    try {
      await ensureChunkListener();
      await deps.translate({
        request_id: requestId,
        text,
        ...langPair,
        engine: "openai",
      });
    } catch (e) {
      if (requestId === aiRequestId) {
        state.aiError = toError(e);
      }
    } finally {
      if (requestId === aiRequestId) {
        state.aiLoading = false;
        refreshLoading();
      }
    }
  }

  async function runText(text: string) {
    const generation = ++runGeneration;
    lastRunText = text;
    activeText = text;
    state.error = null;
    state.webError = null;
    state.aiError = null;
    state.webOutput = "";
    state.aiOutput = "";
    const langPair = resolveLangPair(text);
    const webTask = runWeb(text, state.webEngine, langPair);
    const aiTask = runAi(text, langPair);

    await Promise.allSettled([webTask, aiTask]);
    if (generation !== runGeneration) return;
    refreshLoading();
    if (activeText === text) {
      activeText = "";
    }
    await drainQueuedText();
  }

  async function drainQueuedText() {
    if (drainingQueue || state.loading || !queuedText) return;

    const generation = ++drainGeneration;
    const text = queuedText;
    queuedText = null;
    drainingQueue = true;
    try {
      state.input = text;
      await runText(text);
    } finally {
      if (generation === drainGeneration) {
        drainingQueue = false;
      }
    }

    if (generation !== drainGeneration) return;
    if (queuedText && !state.loading) {
      await drainQueuedText();
    }
  }

  /** 切换网页翻译引擎；若已有输入则只重跑网页侧翻译。 */
  async function setWebEngine(engine: WebEngine) {
    if (state.webEngine === engine) return;
    state.webEngine = engine;

    const inputText = state.input.trim();
    if (!inputText) return;
    if (state.loading && activeText && inputText !== activeText) {
      queueText(inputText);
      state.webOutput = "";
      return;
    }

    const text = activeText || inputText;
    await runWeb(text, engine, resolveLangPair(text));
    await drainQueuedText();
  }

  async function translateText(value: string) {
    const text = value.trim();
    if (!text) return;
    state.input = text;
    if (state.loading) {
      queueText(text);
      return;
    }
    await runText(text);
  }

  async function restartForLanguageChange() {
    const text = state.input.trim();
    clearQueuedText();
    drainGeneration += 1;
    drainingQueue = false;

    if (text) {
      await runText(text);
      return;
    }

    runGeneration += 1;
    webRequestId += 1;
    aiRequestId += 1;
    activeText = "";
    state.webOutput = "";
    state.aiOutput = "";
    state.webError = null;
    state.aiError = null;
    state.webLoading = false;
    state.aiLoading = false;
    refreshLoading();
  }

  async function setSource(source: string) {
    if (state.source === source) return;
    state.source = source;
    await restartForLanguageChange();
  }

  async function setTarget(target: string) {
    if (state.target === target) return;
    state.target = target;
    await restartForLanguageChange();
  }

  function setInput(v: string) {
    state.input = v;
    if (!state.loading) return;

    const text = v.trim();
    if (!text) {
      clearQueuedText();
      return;
    }
    if (text === activeText) {
      clearQueuedText();
      return;
    }
    queueText(text);
  }

  function shouldAutoRun(text: string): boolean {
    return Boolean(text.trim()) && !state.loading && text.trim() !== lastRunText;
  }

  /** 截图 OCR：Apple 自带自由框选 → 识别 → 填入输入框 → 自动翻译。 */
  async function captureAndTranslate() {
    state.error = null;
    try {
      const text = await deps.screenshotOcr();
      if (!text.trim()) return;
      await deps.showMain(); // OCR 有结果后再唤起窗口
      await translateText(text);
    } catch (e) {
      state.error = toError(e);
      await deps.showMain();
    }
  }

  function reset() {
    drainGeneration += 1;
    runGeneration += 1;
    webRequestId += 1;
    aiRequestId += 1;
    activeText = "";
    queuedText = null;
    drainingQueue = false;
    lastRunText = "";
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
    setSource,
    setTarget,
    shouldAutoRun,
    setWebEngine,
    captureAndTranslate,
    reset,
  };
}

export const translation = createTranslationStore();
