import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";
import type { TaskbarDisplay, ThemePreference } from "../types";

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
  );
}
