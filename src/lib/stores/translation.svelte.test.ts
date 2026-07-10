import { describe, expect, test, vi } from "vitest";
import { createTranslationStore } from "./translation.svelte";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeDependencies() {
  type TestChunk = {
    request_id: number | null;
    delta: string;
    done: boolean;
  };
  let chunkHandler: ((chunk: TestChunk) => void) | null = null;
  return {
    translate: vi.fn(),
    screenshotOcr: vi.fn(async () => ""),
    showMain: vi.fn(async () => {}),
    onChunk: vi.fn(async (handler: (chunk: TestChunk) => void) => {
      chunkHandler = handler;
      return vi.fn();
    }),
    emitChunk(chunk: TestChunk) {
      chunkHandler?.(chunk);
    },
  };
}

describe("translation store coordination", () => {
  test("keeps the selected web engine result when an older request finishes later", async () => {
    const deps = fakeDependencies();
    const google = deferred<string>();
    const bing = deferred<string>();
    const ai = deferred<string>();

    deps.translate.mockImplementation((req: { engine: string }) => {
      if (req.engine === "google") return google.promise;
      if (req.engine === "bing") return bing.promise;
      return ai.promise;
    });

    const store = createTranslationStore(deps);
    store.setInput("hello");
    const initialRun = store.run();

    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));
    const switchRun = store.setWebEngine("bing");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(3));

    bing.resolve("Bing result");
    await switchRun;
    google.resolve("Google result");
    ai.resolve("");
    await initialRun;

    expect(store.state.webEngine).toBe("bing");
    expect(store.state.webOutput).toBe("Bing result");
  });

  test("queues the latest selected text while a translation is running", async () => {
    const deps = fakeDependencies();
    const pending = new Map<string, Deferred<string>>();

    deps.translate.mockImplementation((req: { text: string; engine: string }) => {
      const key = `${req.text}:${req.engine}`;
      const task = deferred<string>();
      pending.set(key, task);
      return task.promise;
    });

    const store = createTranslationStore(deps);
    const firstRun = store.translateText("first");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));

    await store.translateText("second");
    expect(store.state.input).toBe("second");
    expect(store.state.error).toContain("排队");

    pending.get("first:google")?.resolve("first web");
    pending.get("first:openai")?.resolve("");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(4));

    pending.get("second:google")?.resolve("second web");
    pending.get("second:openai")?.resolve("");
    await firstRun;

    expect(store.state.input).toBe("second");
    expect(store.state.webOutput).toBe("second web");
    expect(store.state.error).toBeNull();
  });

  test("manual input replaces an older queued translation", async () => {
    const deps = fakeDependencies();
    const pending = new Map<string, Deferred<string>>();

    deps.translate.mockImplementation((req: { text: string; engine: string }) => {
      const key = `${req.text}:${req.engine}`;
      const task = deferred<string>();
      pending.set(key, task);
      return task.promise;
    });

    const store = createTranslationStore(deps);
    const firstRun = store.translateText("first");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));

    await store.translateText("second");
    store.setInput("third");

    pending.get("first:google")?.resolve("first web");
    pending.get("first:openai")?.resolve("");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(4));

    expect(deps.translate).toHaveBeenCalledWith(
      expect.objectContaining({ text: "third", engine: "google" }),
    );
    expect(deps.translate).not.toHaveBeenCalledWith(
      expect.objectContaining({ text: "second", engine: "google" }),
    );

    pending.get("third:google")?.resolve("third web");
    pending.get("third:openai")?.resolve("");
    await firstRun;
  });

  test("captures OCR during an active translation and queues its text", async () => {
    const deps = fakeDependencies();
    const firstWeb = deferred<string>();
    const firstAi = deferred<string>();

    deps.translate.mockImplementation((req: { text: string; engine: string }) => {
      if (req.text === "first" && req.engine === "google") return firstWeb.promise;
      if (req.text === "first" && req.engine === "openai") return firstAi.promise;
      return Promise.resolve(req.engine === "openai" ? "" : "OCR web");
    });
    deps.screenshotOcr.mockResolvedValue("captured text");

    const store = createTranslationStore(deps);
    const firstRun = store.translateText("first");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));

    await store.captureAndTranslate();

    expect(deps.screenshotOcr).toHaveBeenCalledOnce();
    expect(deps.showMain).toHaveBeenCalledOnce();
    expect(store.state.input).toBe("captured text");
    expect(store.state.error).toContain("排队");

    firstWeb.resolve("first web");
    firstAi.resolve("");
    await firstRun;
  });

  test("listener initialization failure clears loading and can be retried", async () => {
    const deps = fakeDependencies();
    deps.onChunk.mockRejectedValueOnce(new Error("listener unavailable"));
    deps.translate.mockResolvedValue("");

    const store = createTranslationStore(deps);
    await store.translateText("first");

    expect(store.state.loading).toBe(false);
    expect(store.state.aiLoading).toBe(false);
    expect(store.state.aiError).toContain("listener unavailable");

    await store.translateText("second");

    expect(deps.onChunk).toHaveBeenCalledTimes(2);
    expect(store.state.loading).toBe(false);
  });

  test("an older invoke completion cannot clear a newer AI loading state", async () => {
    const deps = fakeDependencies();
    const firstAi = deferred<string>();
    const secondAi = deferred<string>();

    deps.translate.mockImplementation((req: { text: string; engine: string }) => {
      if (req.engine !== "openai") return Promise.resolve(`${req.text} web`);
      return req.text === "first" ? firstAi.promise : secondAi.promise;
    });

    const store = createTranslationStore(deps);
    const firstRun = store.translateText("first");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));
    deps.emitChunk({ request_id: 1, delta: "", done: true });
    await vi.waitFor(() => expect(store.state.loading).toBe(false));

    const secondRun = store.translateText("second");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(4));
    expect(store.state.aiLoading).toBe(true);

    firstAi.resolve("");
    await firstRun;

    expect(store.state.aiLoading).toBe(true);

    secondAi.resolve("");
    await secondRun;
  });

  test("ignores stream chunks from a superseded AI request", async () => {
    const deps = fakeDependencies();
    const aiTasks = new Map<number, Deferred<string>>();

    deps.translate.mockImplementation(
      (req: { text: string; engine: string; request_id?: number }) => {
        if (req.engine !== "openai") return Promise.resolve(`${req.text} web`);
        const task = deferred<string>();
        aiTasks.set(req.request_id ?? -1, task);
        return task.promise;
      },
    );

    const store = createTranslationStore(deps);
    const firstRun = store.translateText("first");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));
    const firstRequest = deps.translate.mock.calls.find(
      ([req]) => req.engine === "openai" && req.text === "first",
    )?.[0] as { request_id: number };

    store.reset();
    const secondRun = store.translateText("second");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(4));
    const secondRequest = deps.translate.mock.calls.find(
      ([req]) => req.engine === "openai" && req.text === "second",
    )?.[0] as { request_id: number };

    expect(firstRequest.request_id).not.toBe(secondRequest.request_id);
    deps.emitChunk({
      request_id: firstRequest.request_id,
      delta: "stale",
      done: false,
    });
    expect(store.state.aiOutput).toBe("");

    deps.emitChunk({
      request_id: secondRequest.request_id,
      delta: "fresh",
      done: false,
    });
    expect(store.state.aiOutput).toBe("fresh");

    deps.emitChunk({
      request_id: secondRequest.request_id,
      delta: "",
      done: true,
    });
    aiTasks.get(firstRequest.request_id)?.resolve("");
    aiTasks.get(secondRequest.request_id)?.resolve("");
    await Promise.all([firstRun, secondRun]);
  });

  test("clears a queued replacement when input returns to the active text", async () => {
    const deps = fakeDependencies();
    const firstWeb = deferred<string>();
    const firstAi = deferred<string>();

    deps.translate.mockImplementation((req: { text: string; engine: string }) => {
      if (req.text !== "first") return Promise.resolve("unexpected");
      return req.engine === "openai" ? firstAi.promise : firstWeb.promise;
    });

    const store = createTranslationStore(deps);
    const running = store.translateText("first");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));

    await store.translateText("second");
    store.setInput("first");
    firstWeb.resolve("first web");
    firstAi.resolve("");
    await running;

    expect(deps.translate).toHaveBeenCalledTimes(2);
    expect(store.state.input).toBe("first");
    expect(store.state.webOutput).toBe("first web");
  });

  test("restarts both translations when the target language changes", async () => {
    const deps = fakeDependencies();
    const tasks = new Map<string, Deferred<string>>();
    deps.translate.mockImplementation(
      (req: { text: string; target: string; engine: string }) => {
        const task = deferred<string>();
        tasks.set(`${req.target}:${req.engine}`, task);
        return task.promise;
      },
    );

    const store = createTranslationStore(deps);
    const first = store.translateText("hello");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(2));

    const switched = store.setTarget("ja");
    await vi.waitFor(() => expect(deps.translate).toHaveBeenCalledTimes(4));

    tasks.get("zh-CN:google")?.resolve("stale web");
    tasks.get("zh-CN:openai")?.resolve("");
    tasks.get("ja:google")?.resolve("fresh web");
    tasks.get("ja:openai")?.resolve("");
    await Promise.all([first, switched]);

    expect(store.state.target).toBe("ja");
    expect(store.state.webOutput).toBe("fresh web");
  });
});
