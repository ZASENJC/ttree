/** 应用内更新状态：检查更新、下载、安装并自动重启。
 *
 * 采用 Tauri 官方 Updater 插件：在应用内下载并校验更新包，原地替换 .app，
 * 随后自动重启加载新版本——无需重新安装 DMG 或手动拖拽。
 *
 * 注意：运行中的二进制无法热替换，新代码必须经重启才能生效；
 * 这是 macOS 原生应用的固有约束，重启由本流程自动完成。
 */
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Store } from "@tauri-apps/plugin-store";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "uptodate"
  | "error";

/** 自动检查间隔：7 天（毫秒）。 */
const AUTO_CHECK_INTERVAL_MS = 7 * 24 * 60 * 60 * 1000;
/** 持久化上次自动检查时间的 store 文件（与设置共用 settings.json）。 */
const STORE_FILE = "settings.json";
const LAST_CHECK_KEY = "update.last_check";

class UpdateStore {
  /** 当前安装版本号。 */
  currentVersion = $state("");
  /** 待安装的新版本号；无更新时为空串。 */
  newVersion = $state("");
  status = $state<UpdateStatus>("idle");
  /** 下载进度 0–1。 */
  progress = $state(0);
  /** 用户可读的状态/错误文案。 */
  message = $state("");
  /** 发布说明（来自 latest.json 的 body 字段）。 */
  releaseNotes = $state("");

  private pending: Update | null = null;
  /** 应用常驻期间的周期性检查句柄（兜底：连续运行超过一周时也能触发）。 */
  private periodicTimer: ReturnType<typeof setInterval> | null = null;
  /** 周期检查节流：间隔内最多触发一次实际网络请求。 */
  private static readonly PERIODIC_TICK_MS = 6 * 60 * 60 * 1000;

  /** 拉取当前版本号。 */
  async loadVersion() {
    if (this.currentVersion) {
      return;
    }
    try {
      this.currentVersion = await getVersion();
    } catch {
      this.currentVersion = "";
    }
  }

  /** 后台静默检查（仅发现更新，不自动下载）。命中时进入 available 状态，
   * 用户进设置页可见并自行决定安装。失败不打扰用户。 */
  async autoCheckIfNeeded() {
    this.startPeriodicCheck();
    // 已有可用更新、或正在检查/下载/安装时，不重复检查以免打扰用户。
    if (
      this.status === "available"
      || this.status === "checking"
      || this.status === "downloading"
      || this.status === "installing"
    ) {
      return;
    }
    if (!(await this.shouldAutoCheck())) {
      return;
    }
    await this.checkForUpdates();
    // 仅当「无更新」或「出错」时刷新时间戳；发现新版本时保留旧时间戳，
    // 避免用户暂不安装、下次启动又因满一周被静默刷掉「已是最新」。
    if (this.status === "uptodate" || this.status === "error") {
      await this.markChecked();
    }
  }

  /** 启动常驻期间周期性检查（每 6h 探一次是否满一周），重复启动安全。 */
  startPeriodicCheck() {
    if (this.periodicTimer) {
      return;
    }
    this.periodicTimer = setInterval(() => {
      void this.autoCheckIfNeeded();
    }, UpdateStore.PERIODIC_TICK_MS);
  }

  /** 停止周期性检查（应用卸载/销毁时调用，避免泄漏定时器）。 */
  stopPeriodicCheck() {
    if (this.periodicTimer) {
      clearInterval(this.periodicTimer);
      this.periodicTimer = null;
    }
  }

  /** 距上次检查是否已满一周（或从未检查过）。 */
  private async shouldAutoCheck(): Promise<boolean> {
    try {
      const store = await Store.load(STORE_FILE);
      const last = await store.get<number>(LAST_CHECK_KEY);
      if (!last) {
        return true;
      }
      return Date.now() - last >= AUTO_CHECK_INTERVAL_MS;
    } catch {
      return false;
    }
  }

  /** 记录本次检查时间为 Unix 毫秒。 */
  private async markChecked() {
    try {
      const store = await Store.load(STORE_FILE);
      await store.set(LAST_CHECK_KEY, Date.now());
      await store.save();
    } catch {
      // 持久化失败不影响本次检查结果。
    }
  }

  /** 检查更新；命中则记录待安装版本并返回 true。 */
  async checkForUpdates(): Promise<boolean> {
    // 重入守卫：已在检查/下载/安装，或已发现待安装版本时，复用当前状态，
    // 不重复发起请求，避免覆盖 pending 与 progress 造成状态抖动。
    if (
      this.status === "checking"
      || this.status === "downloading"
      || this.status === "installing"
      || this.status === "available"
    ) {
      return this.status === "available";
    }
    this.status = "checking";
    this.message = "正在检查更新…";
    this.progress = 0;
    this.newVersion = "";
    this.releaseNotes = "";

    try {
      const update = await check();
      if (!update) {
        this.pending = null;
        this.status = "uptodate";
        this.message = "已是最新版本";
        return false;
      }
      this.pending = update;
      this.newVersion = update.version;
      this.releaseNotes = update.body ?? "";
      this.status = "available";
      this.message = `发现新版本 ${update.version}`;
      return true;
    } catch (e) {
      this.pending = null;
      this.status = "error";
      this.message = this.toMessage(e, "检查更新失败，请确认网络可达更新服务器");
      return false;
    }
  }

  /** 下载并安装已发现的更新，完成后自动重启。 */
  async downloadAndInstall() {
    const update = this.pending;
    if (!update) {
      this.status = "error";
      this.message = "没有待安装的更新";
      return;
    }

    try {
      this.status = "downloading";
      this.message = "下载中…";
      this.progress = 0;

      let total = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            this.progress = 0;
            break;
          case "Progress": {
            if (total > 0) {
              const next = (event.data.chunkLength / total) + this.progress;
              this.progress = Math.min(1, next);
            }
            break;
          }
          case "Finished":
            this.progress = 1;
            break;
        }
      });

      this.status = "installing";
      this.message = "安装完成，正在重启…";
      await relaunch();
    } catch (e) {
      this.status = "error";
      this.message = this.toMessage(e, "下载或安装更新失败");
    }
  }

  private toMessage(e: unknown, fallback: string): string {
    if (typeof e === "string" && e.trim()) {
      return e;
    }
    if (e instanceof Error && e.message) {
      return e.message;
    }
    return fallback;
  }
}

export const updater = new UpdateStore();
