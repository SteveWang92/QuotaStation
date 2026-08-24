import { useCallback, useState } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";
import type { AppSettings } from "../types";

/**
 * Which of the things that happen with no window open are worth a desktop notification.
 *
 * Each one is announced when it happens and not again until it has cleared, so leaving all
 * three on is a handful of notifications a day rather than one per refresh.
 */
export function QuotaNotifications() {
  const settings = useAppSettings();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const change = useCallback(async (patch: Partial<AppSettings>) => {
    setBusy(true);
    setError(null);
    try {
      await saveAppSettings(patch);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  return (
    <section className="provider-consent" aria-label="Quota notifications">
      <div className="provider-consent-body">
        <h2>Notify me about quota</h2>
        <p>
          Desktop notifications for what changes while QuotaStation is running with no window open.
          Each is raised once and stays quiet until the condition clears.
        </p>
        {error ? <p className="provider-consent-error">{error}</p> : null}
        <div className="consent-options">
          <label>
            <input
              type="checkbox"
              checked={settings?.notifyLowQuota ?? false}
              disabled={busy || settings === null}
              onChange={(event) => void change({ notifyLowQuota: event.target.checked })}
            />
            A quota window is running low
          </label>
          <label>
            <input
              type="checkbox"
              checked={settings?.notifyReadFailures ?? false}
              disabled={busy || settings === null}
              onChange={(event) => void change({ notifyReadFailures: event.target.checked })}
            />
            A provider cannot be read
          </label>
          <label>
            <input
              type="checkbox"
              checked={settings?.notifyQuotaResets ?? false}
              disabled={busy || settings === null}
              onChange={(event) => void change({ notifyQuotaResets: event.target.checked })}
            />
            A quota window has reset
          </label>
        </div>
      </div>
    </section>
  );
}
