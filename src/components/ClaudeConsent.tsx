import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../errors";

interface ProviderSettings {
  claudeEnabled: boolean;
  claudeConsentGranted: boolean;
}

/**
 * Claude quota is the one acquisition path that presents a stored credential to a remote
 * service, so what that involves is spelled out before it can be switched on for the
 * first time. Once accepted, the tray toggle is enough and this card stays out of the way.
 */
export function ClaudeConsent() {
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

  // The tray toggle cannot turn Claude on by itself the first time, so it sends the user
  // here instead. Without this the dashboard simply appears and the card is easy to miss.
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
        setSettings(await invoke<ProviderSettings>("set_claude_enabled", { enabled, grantConsent }));
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  if (!settings || settings.claudeEnabled) return null;

  return (
    <section
      ref={card}
      className={`provider-consent${requested ? " requested" : ""}`}
      aria-label="Claude Code monitoring"
    >
      <div>
        <h2>Show Claude Code usage</h2>
        {settings.claudeConsentGranted ? (
          <p>Claude Code monitoring is turned off. Turn it back on to see it beside Codex.</p>
        ) : (
          <p>
            Unlike Codex, Claude Code publishes no local quota interface. To show its 5-hour
            and weekly windows, QuotaStation reads the sign-in token Claude Code already
            stored on this machine and sends it to Anthropic's own usage endpoint. The token
            stays in the application core: it is never saved, logged, or shown. Nothing is
            written to your account, and your local history is read without it.
          </p>
        )}
        {error ? <p className="provider-consent-error">{error}</p> : null}
      </div>
      <button
        type="button"
        onClick={() => void setEnabled(true, !settings.claudeConsentGranted)}
        disabled={busy}
      >
        {settings.claudeConsentGranted ? "Turn on" : "Enable Claude Code"}
      </button>
    </section>
  );
}
