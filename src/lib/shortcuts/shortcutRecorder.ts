export type ShortcutField =
  | "toggle"
  | "ocr"
  | "ai_dialog"
  | "selection_translate"
  | "selection_ai_dialog";

export interface ShortcutDraft {
  toggle: string;
  ocr: string;
  ai_dialog: string;
  selection_translate: string;
  selection_ai_dialog: string;
}

export interface ShortcutKeyboardEvent {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  repeat: boolean;
}

export type ShortcutRecordResult =
  | { status: "recorded"; shortcut: string }
  | { status: "invalid"; message: string }
  | { status: "cancelled" }
  | { status: "pending" };

const MODIFIER_KEYS = new Set([
  "Alt",
  "AltGraph",
  "Control",
  "Meta",
  "Shift",
]);

const MODIFIER_CODES = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "MetaLeft",
  "MetaRight",
  "ShiftLeft",
  "ShiftRight",
]);

const NAMED_KEYS = new Map<string, string>([
  ["ArrowDown", "ArrowDown"],
  ["ArrowLeft", "ArrowLeft"],
  ["ArrowRight", "ArrowRight"],
  ["ArrowUp", "ArrowUp"],
  ["Backspace", "Backspace"],
  ["Delete", "Delete"],
  ["End", "End"],
  ["Enter", "Enter"],
  ["Home", "Home"],
  ["Insert", "Insert"],
  ["PageDown", "PageDown"],
  ["PageUp", "PageUp"],
  ["Space", "Space"],
  ["Tab", "Tab"],
]);

const PUNCTUATION_KEYS = new Map<string, string>([
  ["Backquote", "`"],
  ["Backslash", "\\"],
  ["BracketLeft", "["],
  ["BracketRight", "]"],
  ["Comma", ","],
  ["Equal", "="],
  ["Minus", "-"],
  ["Period", "."],
  ["Quote", "'"],
  ["Semicolon", ";"],
  ["Slash", "/"],
]);

export function recordShortcutFromEvent(
  event: ShortcutKeyboardEvent,
): ShortcutRecordResult {
  if (event.key === "Escape" || event.code === "Escape") {
    return { status: "cancelled" };
  }

  if (event.repeat || isModifierOnlyEvent(event)) {
    return { status: "pending" };
  }

  if (!hasRequiredModifier(event)) {
    return {
      status: "invalid",
      message: "快捷键必须包含 Cmd/Ctrl/Option 中的至少一个修饰键。",
    };
  }

  const mainKey = getMainKey(event.code);
  if (!mainKey) {
    return {
      status: "invalid",
      message: "不支持这个按键，请选择字母、数字、方向键或功能键。",
    };
  }

  return {
    status: "recorded",
    shortcut: [...getModifierParts(event), mainKey].join("+"),
  };
}

export function applyShortcutRecord(
  draft: ShortcutDraft,
  field: ShortcutField,
  result: ShortcutRecordResult,
): ShortcutDraft {
  if (result.status !== "recorded") {
    return draft;
  }

  return {
    ...draft,
    [field]: result.shortcut,
  };
}

export function clearShortcutField(
  draft: ShortcutDraft,
  field: ShortcutField,
): ShortcutDraft {
  return {
    ...draft,
    [field]: "",
  };
}

function hasRequiredModifier(event: ShortcutKeyboardEvent): boolean {
  return event.metaKey || event.ctrlKey || event.altKey;
}

function isModifierOnlyEvent(event: ShortcutKeyboardEvent): boolean {
  return MODIFIER_KEYS.has(event.key) || MODIFIER_CODES.has(event.code);
}

function getModifierParts(event: ShortcutKeyboardEvent): string[] {
  const parts: string[] = [];

  if (event.metaKey) {
    parts.push("CmdOrCtrl");
  }

  if (event.ctrlKey) {
    parts.push("Control");
  }

  if (event.altKey) {
    parts.push("Option");
  }

  if (event.shiftKey) {
    parts.push("Shift");
  }

  return parts;
}

function getMainKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) {
    return code.slice(3);
  }

  if (/^Digit[0-9]$/.test(code)) {
    return code.slice(5);
  }

  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code)) {
    return code;
  }

  return NAMED_KEYS.get(code) ?? PUNCTUATION_KEYS.get(code) ?? null;
}
