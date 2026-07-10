import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { appendChatHistory } from "./tauri";

describe("appendChatHistory", () => {
  beforeEach(() => invoke.mockReset());

  it("passes the stable conversation id when resuming old history", async () => {
    invoke.mockResolvedValue(undefined);
    const round = [
      { role: "user" as const, content: "follow up" },
      { role: "assistant" as const, content: "answer" },
    ];

    await appendChatHistory(round, 7);

    expect(invoke).toHaveBeenCalledWith("append_chat_history", {
      round,
      conversationId: 7,
    });
  });

  it("uses a null id for a newly started conversation", async () => {
    invoke.mockResolvedValue(undefined);

    await appendChatHistory([], null);

    expect(invoke).toHaveBeenCalledWith("append_chat_history", {
      round: [],
      conversationId: null,
    });
  });
});
