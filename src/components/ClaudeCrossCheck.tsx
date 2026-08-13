import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../errors";

interface ProviderSettings {
  claudeCrossCheckEnabled: boolean;
  claudeConsentGranted: boolean;
}

/**
 * The third quota source, and the only one that leaves the machine. Asking Anthropic's
 * usage endpoint costs a stored credential and a share of a rate limit Claude Code itself
 * is already spending, so the trade is explained before it can be taken for the first
 * time. The tray toggle is enough afterwards, but the setting is stated here either way:
 * the tray cannot say why it is off.
 */
export function ClaudeCrossCheck() {
  const [settings, setSettings] = useState<ProviderSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setSettings(await invoke<ProviderSettings>("get_provider_settings"));
    } catch {
      // The dashboard works without this row; the tray toggle remains available.
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const setEnabled = useCallback(
    async (enabled: boolean, grantConsent: boolean) => {
      setBusy(true);
      setError(null);
      try {
        setSettings(
          await invoke<ProviderSettings>("set_claude_cross_check", { enabled, grantConsent }),
        );
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  if (!settings) return null;

  const { claudeCrossCheckEnabled: enabled, claudeConsentGranted: consented } = settings;

  return (
    <section className="provider-consent" aria-label="Claude Code online quota cross-check">
      <div>
        <h2>Check Claude quota online</h2>
        {enabled ? (
          <p>
            QuotaStation also asks Anthropic's usage endpoint for the remaining percentage.
            This is only needed where the status line above is not installed, and it shares a
            rate limit with Claude Code's own usage display, so a reading that fails changes
            nothing and the known windows stay on display.
          </p>
        ) : consented ? (
          <p>
            The online cross-check is off. Claude Code's windows still come from its status
            line and its session logs; turning this back on adds a second opinion on the
            remaining percentage.
          </p>
        ) : (
          <p>
            Claude Code's session logs already give the window that is running and when it
            ends. They do not publish an allowance, so the percentage remaining is unknown
            without asking Anthropic. Turning this on lets QuotaStation read the sign-in token
            Claude Code stored on this machine and present it to Anthropic's usage endpoint.
            The token stays in the application core: it is never saved, logged, or shown, and
            nothing is written to your account. Note that Claude Code reads that same endpoint
            for its own usage display and the rate limit is shared, so the two can crowd each
            other out — a reading that fails changes nothing, and the windows from the logs
            stay on display.
          </p>
        )}
        {error ? <p className="provider-consent-error">{error}</p> : null}
      </div>
      <button
        type="button"
        onClick={() => void setEnabled(!enabled, !enabled && !consented)}
        disabled={busy}
      >
        {enabled ? "Turn off" : consented ? "Turn on" : "Enable cross-check"}
      </button>
    </section>
  );
}
