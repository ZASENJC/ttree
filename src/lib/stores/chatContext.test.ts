import { describe, expect, test } from "vitest";
import type { ChatMessage } from "../api/tauri";
import { CHAT_CONTEXT_LIMIT, recentChatContext } from "./chatContext";

function message(index: number): ChatMessage {
  return {
    role: index % 2 === 0 ? "user" : "assistant",
    content: `message-${index}`,
  };
}

describe("recentChatContext", () => {
  test("returns all messages when under the context limit", () => {
    const messages = Array.from({ length: 4 }, (_, index) => message(index));

    const context = recentChatContext(messages);

    expect(context).toEqual(messages);
  });

  test("keeps only the latest messages when history exceeds the context limit", () => {
    const messages = Array.from({ length: CHAT_CONTEXT_LIMIT + 6 }, (_, index) =>
      message(index),
    );

    const context = recentChatContext(messages);

    expect(context).toHaveLength(CHAT_CONTEXT_LIMIT);
    expect(context[0].content).toBe("message-6");
    expect(context.at(-1)?.content).toBe(`message-${CHAT_CONTEXT_LIMIT + 5}`);
  });

  test("drops a leading orphan assistant message after trimming", () => {
    const messages = Array.from({ length: CHAT_CONTEXT_LIMIT + 5 }, (_, index) =>
      message(index),
    );

    const context = recentChatContext(messages);

    expect(context[0].role).toBe("user");
    expect(context[0].content).toBe("message-6");
    expect(context.at(-1)?.content).toBe(`message-${CHAT_CONTEXT_LIMIT + 4}`);
  });

  test("returns empty array when the only surviving message is an orphan assistant", () => {
    // 切窗后首条恰好是 assistant（异常但需容错）：应被丢弃，结果为空数组。
    const messages: ChatMessage[] = [
      { role: "assistant", content: "orphan" },
      { role: "assistant", content: "orphan-2" },
    ];

    const context = recentChatContext(messages);

    expect(context).toEqual([]);
  });
});
