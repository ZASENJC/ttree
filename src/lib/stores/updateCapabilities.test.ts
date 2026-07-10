import { describe, expect, test } from "vitest";
import capability from "../../../src-tauri/capabilities/default.json";

describe("updater store capability", () => {
  test("grants only the store operations used by the updater timestamp", () => {
    expect(
      capability.permissions.filter((permission) => permission.startsWith("store:")),
    ).toEqual([
      "store:allow-load",
      "store:allow-get",
      "store:allow-set",
      "store:allow-save",
    ]);
  });

  test("grants only the updater and process commands used by the app", () => {
    expect(
      capability.permissions.filter((permission) =>
        permission.startsWith("updater:"),
      ),
    ).toEqual([
      "updater:allow-check",
      "updater:allow-download-and-install",
    ]);
    expect(
      capability.permissions.filter((permission) =>
        permission.startsWith("process:"),
      ),
    ).toEqual(["process:allow-restart"]);
  });

  test("does not expose unused positioner commands or redundant autostart defaults", () => {
    expect(
      capability.permissions.filter((permission) =>
        permission.startsWith("positioner:"),
      ),
    ).toEqual([]);
    expect(
      capability.permissions.filter((permission) =>
        permission.startsWith("autostart:"),
      ),
    ).toEqual([
      "autostart:allow-enable",
      "autostart:allow-disable",
      "autostart:allow-is-enabled",
    ]);
  });
});
