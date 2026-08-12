import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../errors";

interface ProviderSettings {
  claudeCrossCheckEnabled: boolean;
  claudeConsentGranted: boolean;
}

/**
 * Claude Code's session logs give the window that is running and when it ends, but never
 * how much of it is left. Anthropic's usage endpoint is the only source for that, and
 * asking it costs a stored credential and a share of a rate limit Claude Code itself is
 * already using. That trade is explained before it can be taken for the first time; once
 * accepted, the tray toggle is enough and this card stays out of the way.
 */
export function ClaudeCrossCheck() {
  const [settings, setSettings] = useState<ProviderSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [requested, setRequested] = useState(false);
  const card = useRef<HTMLElement | null>(null);

  const load = useCallback(async () => {
    try {
      setSettings(await invoke<ProviderSettings>("get_provider_settings"));
    } catch {
      // The dashboard works without this card; the tray toggle remains available.
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // The tray toggle cannot turn the cross-check on by itself the first time, so it sends
  // the user here instead. Without this the dashboard simply appears and the card is easy
  // to miss.
  useEffect(() => {
    let disposed = false;
    let stopListening = () => {};
    void listen("claude-consent-requested", () => {
      setRequested(true);
      void load();
      card.current?.scrollIntoView({ behavior: "smooth", block: "center" });
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening();
    };
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

  if (!settings || settings.claudeCrossCheckEnabled) return null;

  return (
    <section
      ref={card}
      className={`provider-consent${requested ? " requested" : ""}`}
      aria-label="Claude Code online quota cross-check"
    >
      <div>
        <h2>Check Claude quota online</h2>
        {settings.claudeConsentGranted ? (
          <p>
            The online cross-check is off. Claude Code's windows still come from its session
            logs; turning this back on adds the remaining percentage to them.
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
        onClick={() => void setEnabled(true, !settings.claudeConsentGranted)}
        disabled={busy}
      >
        {settings.claudeConsentGranted ? "Turn on" : "Enable cross-check"}
      </button>
    </section>
  );
}
