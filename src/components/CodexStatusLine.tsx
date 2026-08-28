import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";

interface CodexStatusLineStatus {
  configured: boolean;
  statusLine: string[];
  terminalTitle: string[];
}

/** Codex exposes native ordered status items rather than a command hook. */
export function CodexStatusLine() {
  const settings = useAppSettings();
  const [status, setStatus] = useState<CodexStatusLineStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<CodexStatusLineStatus>("get_codex_status_line").then(setStatus);
  }, []);

  const change = async (patch: Record<string, boolean>) => {
    setBusy(true);
    setError(null);
    try {
      await saveAppSettings(patch);
      setStatus(await invoke<CodexStatusLineStatus>("get_codex_status_line"));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="provider-consent" aria-label="Codex status line settings">
      <div className="provider-consent-body">
        <h2>Configure Codex status line</h2>
        <p>
          Codex uses its own native footer, not a command hook. Project name stays in the terminal
          title so the one footer row can show branch, context, and quota.
        </p>
        {error ? <p className="provider-consent-error">{error}</p> : null}
        <div className="consent-options">
          <label>
            <input
              type="checkbox"
              checked={settings?.codexStatusLineEnabled ?? false}
              disabled={busy || settings === null}
              onChange={(event) => void change({ codexStatusLineEnabled: event.target.checked })}
            />
            Let QuotaStation manage Codex status line
          </label>
          <label>
            <input
              type="checkbox"
              checked={settings?.codexStatusLineUpdateDisplay ?? false}
              disabled={busy || settings === null || !settings.codexStatusLineEnabled}
              onChange={(event) =>
                void change({ codexStatusLineUpdateDisplay: event.target.checked })
              }
            />
            Change Codex display items and positions
          </label>
        </div>
        {status?.configured ? (
          <p className="provider-consent-note">
            Footer: {status.statusLine.join(" · ") || "Codex default"}. Title:{" "}
            {status.terminalTitle.join(" · ") || "Codex default"}.
          </p>
        ) : null}
      </div>
    </section>
  );
}
