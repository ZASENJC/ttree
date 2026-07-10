import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  append: vi.fn(),
  chat: vi.fn(),
  clear: vi.fn(),
  load: vi.fn(),
  onChunk: vi.fn(),
  start: vi.fn(),
}));

vi.mock("../api/tauri", () => ({
  appendChatHistory: api.append,
  chat: api.chat,
  clearChatHistory: api.clear,
  loadConversations: api.load,
  onChatChunk: api.onChunk,
  startNewConversation: api.start,
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("chat history persistence", () => {
  beforeEach(() => {
    vi.resetModules();
    Object.values(api).forEach((mock) => mock.mockReset());
  });

  it("persists a follow-up against the opened historical conversation", async () => {
    const conversations = [
      {
        id: 0,
        summary: "A",
        messages: [
          { role: "user" as const, content: "A1" },
          { role: "assistant" as const, content: "A2" },
        ],
      },
      {
        id: 1,
        summary: "B",
        messages: [
          { role: "user" as const, content: "B1" },
          { role: "assistant" as const, content: "B2" },
        ],
      },
    ];
    api.load.mockResolvedValue(conversations);
    api.chat.mockResolvedValue(undefined);
    api.append.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    chatStore.openConversation(0);
    chatStore.state.input = "A3";

    await chatStore.send();
    const chunkHandler = api.onChunk.mock.calls[0]?.[0] as (
      chunk: { request_id: null; delta: string; done: boolean },
    ) => void;
    chunkHandler({ request_id: null, delta: "A4", done: false });
    chunkHandler({ request_id: null, delta: "", done: true });

    await vi.waitFor(() => {
      expect(api.append).toHaveBeenCalledWith(
        [
          { role: "user", content: "A3" },
          { role: "assistant", content: "A4" },
        ],
        0,
      );
    });
  });

  it("waits for the new-session marker before sending the first message", async () => {
    let resolveStart!: () => void;
    const start = new Promise<void>((resolve) => {
      resolveStart = resolve;
    });
    api.load.mockResolvedValue([]);
    api.start.mockReturnValue(start);
    api.chat.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    chatStore.newConversation();
    chatStore.state.input = "first message";
    const sending = chatStore.send();

    await vi.waitFor(() => expect(api.start).toHaveBeenCalledOnce());
    expect(api.chat).not.toHaveBeenCalled();

    resolveStart();
    await sending;

    expect(api.chat).toHaveBeenCalledOnce();
  });

  it("recovers when the chunk listener cannot be registered", async () => {
    api.load.mockResolvedValue([]);
    api.chat.mockResolvedValue(undefined);
    api.onChunk
      .mockRejectedValueOnce(new Error("listener unavailable"))
      .mockResolvedValueOnce(() => {});

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    chatStore.state.input = "first";

    await expect(chatStore.send()).resolves.toBeUndefined();
    expect(chatStore.state.loading).toBe(false);
    expect(chatStore.state.error).toContain("listener unavailable");
    expect(api.chat).not.toHaveBeenCalled();

    chatStore.state.input = "second";
    await chatStore.send();

    expect(api.onChunk).toHaveBeenCalledTimes(2);
    expect(api.chat).toHaveBeenCalledOnce();
  });

  it("waits for startup history before sending into the active conversation", async () => {
    const history = deferred<
      Array<{
        id: number;
        summary: string;
        messages: Array<{ role: "user" | "assistant"; content: string }>;
      }>
    >();
    api.load.mockReturnValue(history.promise);
    api.chat.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    const loading = chatStore.loadHistory();
    chatStore.state.input = "follow up";
    const sending = chatStore.send();

    await Promise.resolve();
    expect(api.chat).not.toHaveBeenCalled();

    history.resolve([
      {
        id: 4,
        summary: "existing",
        messages: [
          { role: "user", content: "old question" },
          { role: "assistant", content: "old answer" },
        ],
      },
    ]);
    await Promise.all([loading, sending]);

    expect(chatStore.state.currentId).toBe(4);
    expect(api.chat).toHaveBeenCalledWith([
      { role: "user", content: "old question" },
      { role: "assistant", content: "old answer" },
      { role: "user", content: "follow up" },
    ]);
  });

  it("does not start two chat requests when sends queue behind startup loading", async () => {
    const history = deferred<[]>();
    api.load.mockReturnValue(history.promise);
    api.chat.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    const loading = chatStore.loadHistory();
    const first = chatStore.sendText("first");
    const second = chatStore.sendText("second");

    history.resolve([]);
    await Promise.all([loading, first, second]);

    expect(api.chat).toHaveBeenCalledOnce();
    expect(chatStore.state.input).toBe("second");
    expect(chatStore.state.error).toContain("仍在生成");
  });

  it("does not start two chat requests behind a new-session marker", async () => {
    const marker = deferred<void>();
    api.load.mockResolvedValue([]);
    api.start.mockReturnValue(marker.promise);
    api.chat.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    chatStore.newConversation();
    const first = chatStore.sendText("first");
    const second = chatStore.sendText("second");

    marker.resolve();
    await Promise.all([first, second]);

    expect(api.chat).toHaveBeenCalledOnce();
    expect(chatStore.state.input).toBe("second");
  });

  it("serializes clear behind startup loading", async () => {
    const history = deferred<
      Array<{
        id: number;
        summary: string;
        messages: Array<{ role: "user" | "assistant"; content: string }>;
      }>
    >();
    api.load.mockReturnValue(history.promise);
    api.clear.mockResolvedValue(undefined);

    const { chatStore } = await import("./chat.svelte");
    const loading = chatStore.loadHistory();
    const clearing = chatStore.clearHistory();

    await Promise.resolve();
    expect(api.clear).not.toHaveBeenCalled();

    history.resolve([
      {
        id: 2,
        summary: "existing",
        messages: [{ role: "user", content: "old" }],
      },
    ]);
    await Promise.all([loading, clearing]);

    expect(api.clear).toHaveBeenCalledOnce();
    expect(chatStore.state.conversations).toEqual([]);
    expect(chatStore.state.messages).toEqual([]);
  });

  it("preserves the visible history when backend clearing fails", async () => {
    const conversations = [
      {
        id: 3,
        summary: "keep me",
        messages: [
          { role: "user" as const, content: "question" },
          { role: "assistant" as const, content: "answer" },
        ],
      },
    ];
    api.load.mockResolvedValue(conversations);
    api.clear.mockRejectedValue(new Error("disk denied"));

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    await chatStore.clearHistory();
    await vi.waitFor(() => expect(chatStore.state.error).toContain("disk denied"));

    expect(chatStore.state.conversations).toEqual(conversations);
    expect(chatStore.state.messages).toEqual(conversations[0].messages);
    expect(chatStore.state.currentId).toBe(3);
  });

  it("retries history loading after a transient failure", async () => {
    api.load
      .mockRejectedValueOnce(new Error("temporary read failure"))
      .mockResolvedValueOnce([
        {
          id: 8,
          summary: "recovered",
          messages: [{ role: "user", content: "restored" }],
        },
      ]);

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    expect(chatStore.state.loaded).toBe(false);

    await chatStore.loadHistory();

    expect(api.load).toHaveBeenCalledTimes(2);
    expect(chatStore.state.loaded).toBe(true);
    expect(chatStore.state.currentId).toBe(8);
  });

  it("keeps blocking sends after a new-session marker fails", async () => {
    api.load.mockResolvedValue([]);
    api.start.mockRejectedValue(new Error("marker write failed"));
    api.chat.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    chatStore.newConversation();

    await chatStore.sendText("first");
    await chatStore.sendText("second");

    expect(api.chat).not.toHaveBeenCalled();
    expect(chatStore.state.input).toBe("second");
    expect(chatStore.state.error).toContain("重新");
  });

  it("does not clear history while a send is waiting for the session marker", async () => {
    const marker = deferred<void>();
    api.load.mockResolvedValue([]);
    api.start.mockReturnValue(marker.promise);
    api.clear.mockResolvedValue(undefined);
    api.chat.mockResolvedValue(undefined);
    api.onChunk.mockResolvedValue(() => {});

    const { chatStore } = await import("./chat.svelte");
    await chatStore.loadHistory();
    chatStore.newConversation();
    const sending = chatStore.sendText("pending");

    await chatStore.clearHistory();

    expect(api.clear).not.toHaveBeenCalled();
    expect(chatStore.state.error).toContain("正在");

    marker.resolve();
    await sending;
  });
});
