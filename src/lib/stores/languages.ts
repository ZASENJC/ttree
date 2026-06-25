/** 支持的语言列表。 */
export interface Lang {
  code: string;
  name: string;
}

/** 自适应目标：根据输入在中/英之间自动切换。 */
export const ADAPTIVE_TARGET = "adaptive";

export const LANGUAGES: Lang[] = [
  { code: "auto", name: "自动检测" },
  { code: "zh-CN", name: "中文（简体）" },
  { code: "zh-TW", name: "中文（繁体）" },
  { code: "en", name: "英语" },
  { code: "ja", name: "日语" },
  { code: "ko", name: "韩语" },
  { code: "fr", name: "法语" },
  { code: "de", name: "德语" },
  { code: "es", name: "西班牙语" },
  { code: "ru", name: "俄语" },
];

/** 目标语言：首项为自适应，其余不含 auto。 */
export const TARGET_LANGUAGES: Lang[] = [
  { code: ADAPTIVE_TARGET, name: "自适应 中⇄英" },
  ...LANGUAGES.filter((l) => l.code !== "auto"),
];

export function langName(code: string): string {
  if (code === ADAPTIVE_TARGET) return "自适应 中⇄英";
  return LANGUAGES.find((l) => l.code === code)?.name ?? code;
}

/**
 * 自适应目标策略：主语言为中/英。
 * 输入含中日韩表意文字（以中文为主）时翻译为英文，否则翻译为中文。
 */
export function resolveAdaptiveTarget(text: string): "en" | "zh-CN" {
  return containsChinese(text) ? "en" : "zh-CN";
}

/** 文本是否包含中文字符（CJK 统一表意文字基本区）。 */
function containsChinese(text: string): boolean {
  return /[一-鿿]/.test(text);
}
