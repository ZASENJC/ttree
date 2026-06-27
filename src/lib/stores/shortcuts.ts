/** 快捷键展示标签（与后端 shortcut.rs 默认值保持一致）。 */
import { isWindows, MODIFIER, SHIFT } from "../platform";

export const shortcut = {
  toggle: isWindows ? "Alt+Space" : `${MODIFIER}${SHIFT}Space`,
  ocr: isWindows ? "" : `${MODIFIER}${SHIFT}S`,
} as const;
