import { describe, expect, test } from "vitest";
import { clampSourceHeight } from "./panelSizing";

describe("clampSourceHeight", () => {
  test("shrinks the source pane when the window is restored at 300px high", () => {
    expect(clampSourceHeight(210, 300)).toBe(88);
  });

  test("keeps the default source height at the normal 460px window height", () => {
    expect(clampSourceHeight(210, 460)).toBe(210);
  });
});
