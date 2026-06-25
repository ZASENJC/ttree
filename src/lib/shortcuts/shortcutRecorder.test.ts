import { describe, expect, test } from "vitest";
import {
  applyShortcutRecord,
  clearShortcutField,
  recordShortcutFromEvent,
  type ShortcutDraft,
  type ShortcutKeyboardEvent,
} from "./shortcutRecorder";

function keyboardEvent(
  overrides: Partial<ShortcutKeyboardEvent>,
): ShortcutKeyboardEvent {
  return {
    key: "s",
    code: "KeyS",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    ...overrides,
  };
}

describe("recordShortcutFromEvent", () => {
  test("records command shift space in the backend shortcut format", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: " ",
        code: "Space",
        metaKey: true,
        shiftKey: true,
      }),
    );

    expect(result).toEqual({
      status: "recorded",
      shortcut: "CmdOrCtrl+Shift+Space",
    });
  });

  test("records option space", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: " ",
        code: "Space",
        altKey: true,
      }),
    );

    expect(result).toEqual({ status: "recorded", shortcut: "Option+Space" });
  });

  test("normalizes letter keys from the physical keyboard code", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "s",
        code: "KeyS",
        metaKey: true,
        shiftKey: true,
      }),
    );

    expect(result).toEqual({ status: "recorded", shortcut: "CmdOrCtrl+Shift+S" });
  });

  test("outputs modifiers in a stable order", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "k",
        code: "KeyK",
        shiftKey: true,
        altKey: true,
        ctrlKey: true,
        metaKey: true,
      }),
    );

    expect(result).toEqual({
      status: "recorded",
      shortcut: "CmdOrCtrl+Control+Option+Shift+K",
    });
  });

  test("records function keys", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({ key: "F1", code: "F1", metaKey: true }),
    );

    expect(result).toEqual({ status: "recorded", shortcut: "CmdOrCtrl+F1" });
  });

  test("records arrow keys", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({ key: "ArrowUp", code: "ArrowUp", metaKey: true }),
    );

    expect(result).toEqual({
      status: "recorded",
      shortcut: "CmdOrCtrl+ArrowUp",
    });
  });

  test("records named editing keys", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({ key: "Enter", code: "Enter", ctrlKey: true }),
    );

    expect(result).toEqual({ status: "recorded", shortcut: "Control+Enter" });
  });

  test("keeps waiting when the user only presses a modifier key", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "Shift",
        code: "ShiftLeft",
        shiftKey: true,
      }),
    );

    expect(result).toEqual({ status: "pending" });
  });

  test("rejects shift-only shortcuts", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "s",
        code: "KeyS",
        shiftKey: true,
      }),
    );

    expect(result.status).toBe("invalid");
    if (result.status === "invalid") {
      expect(result.message).toContain("Cmd/Ctrl/Option");
    }
  });

  test("rejects main keys without a modifier", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({ key: "s", code: "KeyS" }),
    );

    expect(result.status).toBe("invalid");
    if (result.status === "invalid") {
      expect(result.message).toContain("修饰键");
    }
  });

  test("cancels recording with Escape", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "Escape",
        code: "Escape",
        metaKey: true,
      }),
    );

    expect(result).toEqual({ status: "cancelled" });
  });

  test("ignores repeated keydown events", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "s",
        code: "KeyS",
        metaKey: true,
        repeat: true,
      }),
    );

    expect(result).toEqual({ status: "pending" });
  });

  test("rejects unsupported physical keys", () => {
    const result = recordShortcutFromEvent(
      keyboardEvent({
        key: "¥",
        code: "IntlYen",
        metaKey: true,
      }),
    );

    expect(result.status).toBe("invalid");
    if (result.status === "invalid") {
      expect(result.message).toContain("不支持");
    }
  });
});

describe("applyShortcutRecord", () => {
  const draft: ShortcutDraft = {
    toggle: "CmdOrCtrl+Shift+Space",
    ocr: "CmdOrCtrl+Shift+S",
    ai_dialog: "",
    selection_translate: "",
    selection_ai_dialog: "",
  };

  test("updates only the selected shortcut field", () => {
    const updated = applyShortcutRecord(draft, "ocr", {
      status: "recorded",
      shortcut: "Option+Space",
    });

    expect(updated).toEqual({
      toggle: "CmdOrCtrl+Shift+Space",
      ocr: "Option+Space",
      ai_dialog: "",
      selection_translate: "",
      selection_ai_dialog: "",
    });
    expect(draft).toEqual({
      toggle: "CmdOrCtrl+Shift+Space",
      ocr: "CmdOrCtrl+Shift+S",
      ai_dialog: "",
      selection_translate: "",
      selection_ai_dialog: "",
    });
  });

  test("updates only the AI dialog shortcut field", () => {
    const updated = applyShortcutRecord(draft, "ai_dialog", {
      status: "recorded",
      shortcut: "CmdOrCtrl+Shift+A",
    });

    expect(updated).toEqual({
      toggle: "CmdOrCtrl+Shift+Space",
      ocr: "CmdOrCtrl+Shift+S",
      ai_dialog: "CmdOrCtrl+Shift+A",
      selection_translate: "",
      selection_ai_dialog: "",
    });
    expect(draft.ai_dialog).toBe("");
  });

  test("leaves the draft unchanged for invalid input", () => {
    const updated = applyShortcutRecord(draft, "toggle", {
      status: "invalid",
      message: "快捷键必须包含至少一个修饰键",
    });

    expect(updated).toBe(draft);
  });

  test("leaves the draft unchanged for pending and cancelled input", () => {
    expect(applyShortcutRecord(draft, "ai_dialog", { status: "pending" })).toBe(
      draft,
    );
    expect(applyShortcutRecord(draft, "ai_dialog", { status: "cancelled" })).toBe(
      draft,
    );
  });

  test("clears only the selected shortcut field", () => {
    const configured: ShortcutDraft = {
      ...draft,
      ai_dialog: "CmdOrCtrl+Shift+A",
    };

    expect(clearShortcutField(configured, "ai_dialog")).toEqual({
      toggle: "CmdOrCtrl+Shift+Space",
      ocr: "CmdOrCtrl+Shift+S",
      ai_dialog: "",
      selection_translate: "",
      selection_ai_dialog: "",
    });
  });

  test("clears a selection shortcut field independently", () => {
    const configured: ShortcutDraft = {
      ...draft,
      selection_translate: "CmdOrCtrl+Shift+T",
      selection_ai_dialog: "CmdOrCtrl+Shift+D",
    };

    expect(clearShortcutField(configured, "selection_translate")).toEqual({
      toggle: "CmdOrCtrl+Shift+Space",
      ocr: "CmdOrCtrl+Shift+S",
      ai_dialog: "",
      selection_translate: "",
      selection_ai_dialog: "CmdOrCtrl+Shift+D",
    });
  });
});
