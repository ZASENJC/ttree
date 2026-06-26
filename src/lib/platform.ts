/**
 * 平台检测工具 —— 根据 navigator.userAgent 判断当前运行平台。
 * 用于前端在 UI 层面做平台自适应（快捷键符号、字体、权限提示等）。
 */

const ua = navigator.userAgent;

export const isMacOS = ua.includes("Mac");
export const isWindows = ua.includes("Win");

/** 修饰键显示符号：macOS 用 ⌘，其他平台用 Ctrl */
export const MODIFIER = isMacOS ? "⌘" : "Ctrl";
/** Shift 显示符号：macOS 用 ⇧，其他平台用 Shift */
export const SHIFT = isMacOS ? "⇧" : "Shift";
/** Option/Alt 显示符号：macOS 用 ⌥，其他平台用 Alt */
export const OPTION = isMacOS ? "⌥" : "Alt";
