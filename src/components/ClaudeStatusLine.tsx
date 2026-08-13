import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../errors";
import { formatResetTimestamp } from "../format";
import type { ClaudeStatusLineStatus } from "../types";

/**
 * Claude Code reports both of its quota windows — the five-hour and the seven-day one, each
 * with the percentage consumed and the exact restart — to whatever command is configured as
 * its status line, and to nothing else. Registering QuotaStation as that command is
 * therefore the only way to see the seven-day window without presenting a credential to
 * Anthropic, and it costs nothing at all: no token, no network, no rate limit shared with
 * Claude Code's own usage display.
 *
 * It does change a setting in Claude Code's own configuration, so it is never installed
 * without being asked for here, and a status line belonging to something else is reported
 * rather than replaced.
 */
export function ClaudeStatusLine() {
  const [status, setStatus] = useState<ClaudeStatusLineStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setStatus(await invoke<ClaudeStatusLineStatus>("get_claude_status_line"));
    } catch {
      // The quota still comes from the session logs; this card is an offer, not a step.
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const setInstalled = useCallback(async (installed: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await invoke<ClaudeStatusLineStatus>("set_claude_status_line", { installed }));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  if (!status) return null;

  return (
    <section className="provider-consent" aria-label="Claude Code status line quota source">
      <div>
        <h2>Read Claude quota from Claude Code</h2>
        {status.installed ? (
          <p>
            QuotaStation is registered as Claude Code's status line, which is how Claude Code
            hands over its five-hour and seven-day windows.{" "}
            {status.lastReadingAt === null
              ? "No reading has arrived yet — the first one comes with the next Claude Code turn."
              : `Last reading ${formatResetTimestamp(status.lastReadingAt)}.`}
          </p>
        ) : (
          <p>
            Claude Code's session logs give the five-hour window's timing but never an
            allowance, and they say nothing at all about the seven-day window. Claude Code
            does report both, with the percentage consumed and the exact restart, to whatever
            command is set as its status line. Installing this registers QuotaStation as that
            command: no credential is read, nothing leaves this machine, and Claude Code
            shows the same two windows in its own status line. The readings arrive while
            Claude Code is running; between sessions the windows above stay as last reported.
          </p>
        )}
        {status.foreignCommand ? (
          <p className="provider-consent-error">
            Claude Code already runs its own status line, which QuotaStation will not
            replace. Remove it in Claude Code's settings first.
          </p>
        ) : null}
        {error ? <p className="provider-consent-error">{error}</p> : null}
      </div>
      <button
        type="button"
        onClick={() => void setInstalled(!status.installed)}
        disabled={busy || (!status.installed && status.foreignCommand !== null)}
      >
        {status.installed ? "Remove status line" : "Install status line"}
      </button>
    </section>
  );
}
