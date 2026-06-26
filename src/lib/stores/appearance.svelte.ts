/** 外观状态：面板透明度等。在应用根上设置 CSS 变量，供各面板背景引用。 */
import { getAppearanceConfig } from "../api/tauri";

const ROOT_VAR = "--panel-opacity";
/** 透明度下限与后端 config.rs PANEL_OPACITY_MIN(0.35) 对齐，避免前后端钳制范围不一致。 */
const OPACITY_MIN = 0.35;
const OPACITY_MAX = 1;

class AppearanceStore {
  /** 面板透明度 0–1；默认完全不透明。 */
  opacity = $state(1);
  loaded = $state(false);

  /** 拉取持久化的外观配置，并应用到 document root。 */
  async load() {
    try {
      const cfg = await getAppearanceConfig();
      this.set(cfg.panel_opacity);
    } catch {
      this.set(1);
    }
    this.loaded = true;
  }

  /** 更新透明度并实时应用到 DOM（不落盘，由调用方决定何时保存）。 */
  set(opacity: number) {
    const clamped = Math.min(OPACITY_MAX, Math.max(OPACITY_MIN, opacity));
    this.opacity = clamped;
    if (typeof document !== "undefined") {
      document.documentElement.style.setProperty(ROOT_VAR, String(clamped));
    }
  }
}

export const appearance = new AppearanceStore();
