import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";
import { formatResetTimestamp } from "../format";
import type { AppSettings, ClaudeStatusLineStatus, ProviderLabelStyle } from "../types";

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
  const settings = useAppSettings();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);

  useEffect(() => {
    void invoke<ClaudeStatusLineStatus>("get_claude_status_line")
      .then(setStatus)
      .catch(() => {
        // The quota still comes from the session logs; this card is an offer, not a step.
      });
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

  const change = useCallback(async (patch: Partial<AppSettings>) => {
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
          <div className="consent-options">
            <label>
              <input
                type="checkbox"
                checked={settings.statusLineOtherProviders}
                disabled={savingSettings}
                onChange={(event) =>
                  void change({ statusLineOtherProviders: event.target.checked })
                }
              />
              Show the other providers' usage, not only Claude's own windows
            </label>
            {/* Claude Code has no status line of its own to fall back to, so turning this
                off leaves the model and the quota — as close to installing nothing as an
                installed status line gets. */}
            <label>
              <input
                type="checkbox"
                checked={settings.statusLineExtraDetails}
                disabled={savingSettings}
                onChange={(event) => void change({ statusLineExtraDetails: event.target.checked })}
              />
              Show what Claude Code does not: the project, branch, context, cache and cost
            </label>
            <label>
              Provider names
              <select
                value={settings.statusLineProviderLabels}
                disabled={savingSettings}
                onChange={(event) =>
                  void change({
                    statusLineProviderLabels: event.target.value as ProviderLabelStyle,
                  })
                }
              >
                <option value="short">Short (CDX, CLD)</option>
                <option value="full">Full (Codex, Claude Code)</option>
              </select>
            </label>
          </div>
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
    void invoke<boolean>("get_claude_notifications")
      .then(setInstalled)
      .catch(() => {});
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
          <code>Stop</code> hook to Claude Code's settings and leaves every other hook alone.
          Nothing from the conversation is read or stored — the notification names the project
          directory and nothing else.
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
 * What installing actually does, at the moment it is being decided. The same words sat
 * permanently on the card before, where they were a wall of text in front of a setting
 * most people had already made up their mind about.
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
