/** 快捷键展示标签（与后端 shortcut.rs 默认值保持一致）。 */
import { MODIFIER, SHIFT } from "../platform";

export const shortcut = {
  toggle: `${MODIFIER}${SHIFT}Space`,
  ocr: `${MODIFIER}${SHIFT}S`,
} as const;
