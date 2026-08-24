import { invoke } from "@tauri-apps/api/core";
import { documentDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";
import type { AppSettings, TaskbarDisplay, ThemePreference } from "../types";

/**
 * How the application sits on the machine: whether Windows starts it, whether it draws the
 * taskbar status, and the desktop shortcut.
 *
 * These were tray menu items, which put them where they could only be found by right
 * clicking an icon, and where a failure had nowhere to be reported — the menu closed and the
 * reason went to the log. They belong beside the other preferences.
 */
export function GeneralSettings() {
  const settings = useAppSettings();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [shortcutCreated, setShortcutCreated] = useState(false);
  const [displays, setDisplays] = useState<TaskbarDisplay[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [sharingError, setSharingError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<boolean>("get_autostart").then(setAutostart);
    void invoke<TaskbarDisplay[]>("get_taskbar_displays").then(setDisplays);
  }, []);

  const changeAutostart = useCallback(async (enabled: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setAutostart(await invoke<boolean>("set_autostart", { enabled }));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const changeTheme = useCallback(async (theme: ThemePreference) => {
    setBusy(true);
    setError(null);
    try {
      await saveAppSettings({ theme });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const changeTaskbarWidget = useCallback(async (enabled: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await saveAppSettings({ taskbarWidgetEnabled: enabled });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const changeTaskbarDisplay = useCallback(async (display: string) => {
    setBusy(true);
    setError(null);
    try {
      await saveAppSettings({ taskbarWidgetDisplay: display === "" ? null : display });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const changeSharing = useCallback(async (patch: Partial<AppSettings>) => {
    setBusy(true);
    setSharingError(null);
    try {
      await saveAppSettings(patch);
    } catch (cause) {
      setSharingError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const chooseSharedFolder = useCallback(async () => {
    setBusy(true);
    setSharingError(null);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose shared usage folder",
        defaultPath: settings?.sharedUsageFolder ?? (await documentDir()),
      });
      if (selected !== null) await saveAppSettings({ sharedUsageFolder: selected });
    } catch (cause) {
      setSharingError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [settings?.sharedUsageFolder]);

  const createShortcut = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("create_desktop_shortcut");
      setShortcutCreated(true);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  return (
    <>
      <section className="provider-consent" aria-label="Application settings">
        <div className="provider-consent-body">
          <h2>Application</h2>
          <p>
            Where QuotaStation shows up on this machine. Nothing here reads a provider or leaves the
            local system.
          </p>
          {error ? <p className="provider-consent-error">{error}</p> : null}
          <div className="consent-options">
            <label>
              Theme
              <select
                value={settings?.theme ?? "dark"}
                disabled={busy || settings === null}
                onChange={(event) => void changeTheme(event.target.value as ThemePreference)}
              >
                <option value="system">Follow Windows</option>
                <option value="dark">Dark</option>
                <option value="light">Light</option>
              </select>
            </label>
            <label>
              <input
                type="checkbox"
                checked={autostart ?? false}
                disabled={busy || autostart === null}
                onChange={(event) => void changeAutostart(event.target.checked)}
              />
              Start with Windows
            </label>
            <label>
              <input
                type="checkbox"
                checked={settings?.taskbarWidgetEnabled ?? false}
                disabled={busy || settings === null}
                onChange={(event) => void changeTaskbarWidget(event.target.checked)}
              />
              Show the quota status in the taskbar
            </label>
            {/* Offered only where there is a choice: one display makes the row pure noise. */}
            {displays.length > 1 ? (
              <label>
                Taskbar display
                <select
                  value={settings?.taskbarWidgetDisplay ?? ""}
                  disabled={busy || settings === null || !settings.taskbarWidgetEnabled}
                  onChange={(event) => void changeTaskbarDisplay(event.target.value)}
                >
                  <option value="">Follow the primary taskbar</option>
                  {displays.map((display) => (
                    <option key={display.id} value={display.id}>
                      {display.label}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
          </div>
        </div>
        <button type="button" onClick={() => void createShortcut()} disabled={busy}>
          {shortcutCreated ? "Shortcut created" : "Create desktop shortcut"}
        </button>
      </section>
      <section className="provider-consent" aria-label="Shared usage folder settings">
        <div className="provider-consent-body">
          <h2>Shared usage folder</h2>
          <p>
            Exchange aggregate token and cost totals with your other QuotaStation devices. Leave the
            folder blank to keep usage on this machine only.
          </p>
          {sharingError ? <p className="provider-consent-error">{sharingError}</p> : null}
          <div className="sharing-fields">
            <label className="folder-picker-field">
              Folder path
              <span className="folder-picker">
                <input
                  type="text"
                  value={settings?.sharedUsageFolder ?? ""}
                  placeholder="Not enabled"
                  readOnly
                  disabled={settings === null}
                />
                <button
                  type="button"
                  disabled={busy || settings === null}
                  onClick={() => void chooseSharedFolder()}
                >
                  Choose folder
                </button>
                {settings?.sharedUsageFolder ? (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void changeSharing({ sharedUsageFolder: null })}
                  >
                    Disable
                  </button>
                ) : null}
              </span>
            </label>
            <label>
              This machine's display name
              <input
                type="text"
                key={settings?.deviceName ?? "device-unset"}
                defaultValue={settings?.deviceName ?? ""}
                disabled={busy || settings === null}
                onBlur={(event) => {
                  const deviceName = event.target.value.trim() || null;
                  if (deviceName !== settings?.deviceName) void changeSharing({ deviceName });
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") event.currentTarget.blur();
                }}
              />
            </label>
          </div>
        </div>
      </section>
    </>
  );
}
