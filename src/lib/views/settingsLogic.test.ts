import { afterEach, describe, expect, test, vi } from "vitest";
import {
  buildApiKeySavePayload,
  createDebouncedSaver,
  performUpdateAction,
  saveAndReload,
  toggleBooleanPreference,
} from "./settingsLogic";

describe("settings actions", () => {
  test("checks first and installs only after a second explicit action", async () => {
    const updater = {
      status: "idle",
      checkForUpdates: vi.fn(async () => {
        updater.status = "available";
        return true;
      }),
      downloadAndInstall: vi.fn(async () => {}),
    };

    await performUpdateAction(updater);
    expect(updater.checkForUpdates).toHaveBeenCalledOnce();
    expect(updater.downloadAndInstall).not.toHaveBeenCalled();

    await performUpdateAction(updater);
    expect(updater.downloadAndInstall).toHaveBeenCalledOnce();
  });

  test("returns a visible error when config saving is rejected", async () => {
    const original = { model: "model-a" };
    const load = vi.fn(async () => ({ model: "model-b" }));

    const result = await saveAndReload(
      original,
      vi.fn(async () => {
        throw new Error("invalid base URL");
      }),
      load,
      "保存 AI 翻译配置失败",
    );

    expect(result).toEqual({
      ok: false,
      value: original,
      error: "保存 AI 翻译配置失败: invalid base URL",
    });
    expect(load).not.toHaveBeenCalled();
  });

  test("preserves an existing API key unless clearing is explicit", () => {
    const config = {
      api_key: "",
      has_api_key: true,
      clear_api_key: false,
      model: "model-a",
    };

    expect(buildApiKeySavePayload(config, false).clear_api_key).toBe(false);
    expect(buildApiKeySavePayload(config, true).clear_api_key).toBe(true);
    expect(
      buildApiKeySavePayload({ ...config, api_key: "replacement" }, true)
        .clear_api_key,
    ).toBe(false);
  });

  test("keeps the previous checkbox value when autostart update fails", async () => {
    const result = await toggleBooleanPreference(
      true,
      vi.fn(async () => {
        throw new Error("permission denied");
      }),
      "切换开机自启失败",
    );

    expect(result).toEqual({
      value: true,
      error: "切换开机自启失败: permission denied",
    });
  });
});

describe("debounced settings persistence", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  test("flushes the latest pending value exactly once on teardown", async () => {
    vi.useFakeTimers();
    const save = vi.fn(async () => {});
    const saver = createDebouncedSaver(400, save);

    saver.schedule(0.6);
    saver.schedule(0.7);
    await saver.flush();
    await vi.runAllTimersAsync();

    expect(save).toHaveBeenCalledOnce();
    expect(save).toHaveBeenCalledWith(0.7);
  });
});
