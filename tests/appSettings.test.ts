import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../src/types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const STORED: AppSettings = {
  theme: "system",
  taskbarWidgetEnabled: false,
  taskbarWidgetDisplay: null,
  statusLineProviderLabels: "short",
  statusLineOtherProviders: false,
  statusLineExtraDetails: false,
  notifyLowQuota: true,
  notifyReadFailures: true,
  notifyQuotaResets: true,
  deviceId: "device-1",
  deviceName: "Workshop",
  dismissedResetNotices: ["codex:primary:1800000000"],
  sharedUsageFolder: null,
};

/** The settings module keeps one record for the whole window, so each test needs a fresh one. */
async function loadModule() {
  vi.resetModules();
  return await import("../src/appSettings");
}

/** Answers `set_app_settings` the way the core does: with the record it was handed. */
function acceptSaves() {
  invoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
    if (command === "get_app_settings") return STORED;
    return (payload as { settings: AppSettings }).settings;
  });
}

function savedRecords(): AppSettings[] {
  return invoke.mock.calls
    .filter(([command]) => command === "set_app_settings")
    .map(([, payload]) => (payload as { settings: AppSettings }).settings);
}

beforeEach(() => {
  invoke.mockReset();
});

describe("shared settings record", () => {
  it("sends the whole record so a card never resets the fields it does not own", async () => {
    acceptSaves();
    const settings = await loadModule();
    await settings.reloadAppSettings();

    await settings.saveAppSettings({ notifyLowQuota: false });

    expect(savedRecords()).toEqual([{ ...STORED, notifyLowQuota: false }]);
  });

  it("builds each queued change on the change before it rather than on a stale copy", async () => {
    let releaseFirstSave = () => {};
    const firstSaveStarted = new Promise<void>((resolve) => {
      releaseFirstSave = resolve;
    });
    invoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "get_app_settings") return STORED;
      const { settings } = payload as { settings: AppSettings };
      if (settings.theme === "dark") await firstSaveStarted;
      return settings;
    });
    const settings = await loadModule();
    await settings.reloadAppSettings();

    // Two cards changed before either write returns: the theme card, then the notification
    // card, whose patch is built from whatever the theme change saved.
    const theme = settings.saveAppSettings({ theme: "dark" });
    const notifications = settings.saveAppSettings((current) => ({
      notifyLowQuota: !current.notifyLowQuota,
    }));
    releaseFirstSave();
    await Promise.all([theme, notifications]);

    expect(savedRecords()[1]).toEqual({ ...STORED, theme: "dark", notifyLowQuota: false });
  });

  it("reports a rejected write to the card that made it and keeps the next one working", async () => {
    invoke.mockImplementation(async (command: string, payload?: Record<string, unknown>) => {
      if (command === "get_app_settings") return STORED;
      const { settings } = payload as { settings: AppSettings };
      if (settings.taskbarWidgetEnabled) throw "Settings could not be written";
      return settings;
    });
    const settings = await loadModule();
    await settings.reloadAppSettings();

    await expect(settings.saveAppSettings({ taskbarWidgetEnabled: true })).rejects.toBe(
      "Settings could not be written",
    );
    await settings.saveAppSettings({ notifyReadFailures: false });

    expect(savedRecords()[1]).toEqual({ ...STORED, notifyReadFailures: false });
  });

  it("refuses a change made before the first read landed", async () => {
    acceptSaves();
    const settings = await loadModule();

    await expect(settings.saveAppSettings({ notifyLowQuota: false })).rejects.toThrow(
      "Settings have not loaded",
    );
    expect(savedRecords()).toEqual([]);
  });

  it("can be read again after the first read failed", async () => {
    invoke.mockRejectedValueOnce("Settings unavailable.");
    const settings = await loadModule();
    await settings.reloadAppSettings();

    acceptSaves();
    await settings.reloadAppSettings();
    await settings.saveAppSettings({ notifyQuotaResets: false });

    expect(savedRecords()).toEqual([{ ...STORED, notifyQuotaResets: false }]);
  });
});
