import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";
import { formatResetTimestamp } from "../format";
import type { ClaudeStatusLineStatus } from "../types";
import { StatusLineLayoutEditor } from "./StatusLineLayoutEditor";

/**
 * Registering QuotaStation as Claude Code's status line is the only way to see the
 * seven-day window without presenting a credential to Anthropic, and it costs nothing: no
 * token, no network, no rate limit shared with Claude Code's own usage display.
 *
 * It does change a setting in Claude Code's own configuration, which is someone else's
 * file, so it is never installed without being asked for — and the asking happens in the
 * confirmation below rather than in a paragraph nobody finishes reading.
 */
export function ClaudeStatusLine() {
  const [status, setStatus] = useState<ClaudeStatusLineStatus | null>(null);
  const { settings } = useAppSettings();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);

  useEffect(() => {
    void invoke<ClaudeStatusLineStatus>("get_claude_status_line").then(setStatus);
  }, []);

  const setInstalled = useCallback(async (installed: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await invoke<ClaudeStatusLineStatus>("set_claude_status_line", { installed }));
      setConfirming(false);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const change = useCallback(async (patch: Parameters<typeof saveAppSettings>[0]) => {
    setSavingSettings(true);
    setError(null);
    try {
      await saveAppSettings(patch);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSavingSettings(false);
    }
  }, []);

  if (!status) return null;

  return (
    <section className="provider-consent" aria-label="Claude Code status line quota source">
      <div className="provider-consent-body">
        <h2>Read Claude quota from Claude Code</h2>
        {status.installed ? (
          <p>
            Installed.{" "}
            {status.lastReadingAt === null
              ? "No reading has arrived yet — the first comes with the next Claude Code turn in a terminal."
              : `Last reading ${formatResetTimestamp(status.lastReadingAt)}.`}
          </p>
        ) : (
          <p>
            Claude Code reports its five-hour and seven-day windows only to its status line.
            Installing this reads them, and shows every provider's quota back inside Claude Code.
          </p>
        )}
        {/* Claude Code renders a status line in a terminal and nowhere else, so a
            desktop-hosted session never runs this command however it is configured.
            Without saying so, a correct installation looks like a broken one. */}
        {status.installed && status.desktopOnlySessions ? (
          <p className="provider-consent-note">
            The Claude Code sessions running now are hosted by the desktop application, which
            renders no status line. Run <code>claude</code> in a terminal to bring the percentages
            up to date.
          </p>
        ) : null}
        {status.hasForeignCommand ? (
          <p className="provider-consent-error">
            Claude Code already runs its own status line, which QuotaStation will not replace.
            Remove it in Claude Code's settings first.
          </p>
        ) : null}
        {error ? <p className="provider-consent-error">{error}</p> : null}
        {status.installed && settings ? (
          <StatusLineLayoutEditor settings={settings} disabled={savingSettings} onChange={change} />
        ) : null}
      </div>
      <button
        type="button"
        onClick={() => (status.installed ? void setInstalled(false) : setConfirming(true))}
        disabled={busy || (!status.installed && status.hasForeignCommand)}
      >
        {status.installed ? "Remove status line" : "Install status line"}
      </button>
      {confirming ? (
        <ConfirmInstall
          busy={busy}
          onCancel={() => setConfirming(false)}
          onConfirm={() => void setInstalled(true)}
        />
      ) : null}
    </section>
  );
}

/**
 * Claude Code's own completion notice reaches a handful of terminals, none of them the
 * ordinary Windows ones, so a long turn finishes in silence and is found by going back to
 * look. Claude Code will however run a command when the agent stops, which is enough.
 */
export function ClaudeFinishedNotifications() {
  const [installed, setInstalled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void invoke<boolean>("get_claude_notifications").then(setInstalled);
  }, []);

  const change = useCallback(async (wanted: boolean) => {
    setBusy(true);
    setInstalled(wanted);
    setError(null);
    try {
      setInstalled(await invoke<boolean>("set_claude_notifications", { installed: wanted }));
    } catch (cause) {
      setError(errorMessage(cause));
      setInstalled(!wanted);
    } finally {
      setBusy(false);
    }
  }, []);

  if (installed === null) return null;

  return (
    <section className="provider-consent" aria-label="Claude Code completion notifications">
      <div className="provider-consent-body">
        <h2>Notify me when Claude Code finishes</h2>
        <p>
          A desktop notification when a turn ends, so a long one can be left running. This adds a{" "}
          <code>Stop</code> hook to Claude Code's settings and leaves every other hook alone. No
          prompt or response content is read or stored. Only the project directory name and Claude
          Code session title are kept locally so the notification can identify the turn.
        </p>
        {error ? <p className="provider-consent-error">{error}</p> : null}
      </div>
      <button type="button" onClick={() => void change(!installed)} disabled={busy}>
        {installed ? "Turn off notifications" : "Turn on notifications"}
      </button>
    </section>
  );
}

/**
 * What installing actually does, at the moment it is being decided, rather than as a
 * permanent wall of text on the card.
 */
function ConfirmInstall({
  busy,
  onCancel,
  onConfirm,
}: {
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="confirm-overlay" onMouseDown={onCancel}>
      <div
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-label="Install the Claude Code status line"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <h3>Install the Claude Code status line?</h3>
        <p>
          QuotaStation will add a <code>statusLine</code> command to Claude Code's own
          <code> settings.json</code>, leaving every other setting untouched. A status line
          belonging to something else is never replaced.
        </p>
        <p>
          No credential is read and nothing leaves this machine. Claude Code hands the command its
          five-hour and seven-day windows, and the command prints every provider's quota back into
          Claude Code.
        </p>
        <p>
          Only terminal sessions render a status line — the desktop application draws its own
          interface — so readings arrive while <code>claude</code> runs in a terminal, and between
          those the windows stay as last reported. Removing it here undoes all of it.
        </p>
        <div className="confirm-actions">
          <button type="button" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="confirm-primary" onClick={onConfirm} disabled={busy}>
            Install
          </button>
        </div>
      </div>
    </div>
  );
}
