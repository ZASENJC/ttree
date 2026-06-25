import { describe, expect, test } from "vitest";
import { resolveAdaptiveTarget } from "./languages";

describe("resolveAdaptiveTarget", () => {
  test("returns English target when input contains Chinese", () => {
    expect(resolveAdaptiveTarget("你好，世界")).toBe("en");
    expect(resolveAdaptiveTarget("hello 世界")).toBe("en");
  });

  test("returns Simplified Chinese target for English input", () => {
    expect(resolveAdaptiveTarget("hello world")).toBe("zh-CN");
    expect(resolveAdaptiveTarget("Translate this sentence, please.")).toBe("zh-CN");
  });

  test("returns Simplified Chinese target when no Chinese is detected", () => {
    expect(resolveAdaptiveTarget("12345!?")).toBe("zh-CN");
  });
});
