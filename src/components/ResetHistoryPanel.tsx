import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { errorMessage } from "../errors";
import { formatCompactNumber, formatEarlyBy, formatResetTimestamp } from "../format";
import type { ProviderResetHistory } from "../types";

/**
 * Every quota-window restart QuotaStation has recorded, per provider, newest first.
 *
 * The dashboard shows the last restart of each window because that is what explains the
 * window running now. This is the rest of it — the record is never pruned, so a restart
 * from months ago is still here, and reading it back is the only way to see whether early
 * restarts are a pattern or a one-off.
 */
export function ResetHistoryPanel() {
  const [history, setHistory] = useState<ProviderResetHistory[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<ProviderResetHistory[]>("get_reset_history").then(setHistory, (cause) =>
      setError(errorMessage(cause)),
    );
  }, []);

  if (error !== null) return <p className="provider-consent-error">{error}</p>;
  if (history === null) return <p className="settings-empty">Reading the restart history…</p>;

  const recorded = history.filter((provider) => provider.resets.length > 0);
  if (recorded.length === 0) {
    return (
      <p className="settings-empty">
        No quota-window restart has been recorded yet. One is written whenever a window's counter is
        rebuilt, which QuotaStation can only see while it is running.
      </p>
    );
  }

  return (
    <>
      {recorded.map((provider) => (
        <section className="reset-history" key={provider.provider}>
          <h4>
            {provider.displayName}
            <span>
              {provider.resets.length} recorded,{" "}
              {provider.resets.filter((event) => event.classification === "unplanned").length}{" "}
              possibly early
            </span>
          </h4>
          <ul>
            {provider.resets.map((event) => (
              <li key={`${event.windowKind}-${event.newResetsAt}`} className={event.classification}>
                <time dateTime={new Date(event.anchoredAt * 1000).toISOString()}>
                  {formatResetTimestamp(event.anchoredAt)}
                </time>
                <span>{event.windowLabel}</span>
                <strong>{event.usedPercentBefore.toFixed(0)}% used</strong>
                {/* Usage is stored by the hour, so a window's total is exact in the middle
                    and approximate at its two ends; the tilde is what says so at a glance. */}
                <strong
                  title={
                    event.tokensInWindow === null
                      ? "No hourly usage was recorded for this window"
                      : "Summed from the hours that began inside this window"
                  }
                >
                  {event.tokensInWindow === null
                    ? "—"
                    : `~${formatCompactNumber(event.tokensInWindow)}`}
                </strong>
                <em>
                  {event.classification === "unplanned"
                    ? `possibly ${formatEarlyBy(event.earlyBySeconds)}`
                    : "appears on schedule"}
                </em>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </>
  );
}
