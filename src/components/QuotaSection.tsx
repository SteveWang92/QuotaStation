import type { LimitResetEvent, LimitWindow } from "../types";
import { formatCountdown, formatEarlyBy, formatResetTimestamp } from "../format";

interface QuotaSectionProps {
  /** Display name of the provider these windows belong to, for labels and empty copy. */
  provider: string;
  limits: LimitWindow[];
  earnedResetCount: number | null;
  resets: LimitResetEvent[];
  compact?: boolean;
}

/**
 * A window whose expiry matches a recorded restart is the window that restart began, so
 * the note explains an expiry that otherwise looks like it moved for no reason. Only an
 * possible early restart is worth saying anything about; the scheduled ones are the ordinary
 * case and stay in the history below. The detector is heuristic, so the UI must not present
 * this classification as provider-confirmed fact.
 */
function originOf(limit: LimitWindow, resets: LimitResetEvent[]): LimitResetEvent | undefined {
  return resets.find(
    (event) =>
      event.classification === "unplanned" &&
      event.newResetsAt === limit.resetsAt &&
      event.windowKind === limit.kind,
  );
}

function QuotaRow({ limit, origin }: { limit: LimitWindow; origin?: LimitResetEvent }) {
  const used = limit.usedPercent;
  return (
    // Each window is coloured by its own reading: the provider's status is the loudest of
    // them, and inheriting it would paint an untouched window in the colour of a spent one.
    <div
      className={`quota-row${limit.freshness === "stale" ? " stale" : ""}`}
      style={{ "--quota-status-color": limit.statusColor } as React.CSSProperties}
    >
      {/* The label already names the duration, and which source produced the reading is a
          diagnostic rather than something to read at a glance, so it lives in the settings
          dialog beside the acquisition paths it belongs to. */}
      <div className="quota-label">
        <h2>{limit.label}</h2>
      </div>
      <div className="quota-meter" aria-label={`${limit.label} usage`}>
        <div className="quota-track">
          {used === null ? (
            <span className="unknown" aria-label="Usage unavailable" />
          ) : (
            <span style={{ width: `${Math.min(100, Math.max(0, used))}%` }} />
          )}
        </div>
      </div>
      <div className="quota-percent">
        <strong>{used === null ? "—" : `${used.toFixed(1)}%`}</strong>
        <span>{used === null ? "unavailable" : "used"}</span>
      </div>
      <div className="quota-reset">
        <span>Resets in</span>
        <strong>{formatCountdown(limit.resetsAt)}</strong>
        <time dateTime={limit.resetsAt === null ? undefined : new Date(limit.resetsAt * 1000).toISOString()}>
          {formatResetTimestamp(limit.resetsAt)}
        </time>
      </div>
      {origin ? (
        <p className="quota-origin">
          Possibly restarted early on {formatResetTimestamp(origin.anchoredAt)} — estimated{" "}
          {formatEarlyBy(origin.earlyBySeconds)}, after a {origin.usedPercentBefore.toFixed(0)}% usage reading.
        </p>
      ) : null}
    </div>
  );
}

function ResetHistory({ resets }: { resets: LimitResetEvent[] }) {
  const possibleEarly = resets.filter((event) => event.classification === "unplanned").length;
  return (
    <details className="reset-history">
      <summary>
        Reset history <span>{resets.length} recorded, {possibleEarly} possibly early</span>
      </summary>
      <ul>
        {resets.map((event) => (
          <li key={`${event.windowKind}-${event.newResetsAt}`} className={event.classification}>
            <time dateTime={new Date(event.anchoredAt * 1000).toISOString()}>
              {formatResetTimestamp(event.anchoredAt)}
            </time>
            <span>{event.windowLabel}</span>
            <strong>{event.usedPercentBefore.toFixed(0)}% used</strong>
            <em>
              {event.classification === "unplanned"
                ? `possibly ${formatEarlyBy(event.earlyBySeconds)}`
                : "appears on schedule"}
            </em>
          </li>
        ))}
      </ul>
    </details>
  );
}

export function QuotaSection({ provider, limits, earnedResetCount, resets, compact = false }: QuotaSectionProps) {
  return (
    <section
      className={`quota-section${compact ? " compact" : ""}`}
      aria-label={`${provider} quota windows`}
    >
      {limits.length > 0 ? (
        limits.map((limit) => <QuotaRow key={limit.kind} limit={limit} origin={originOf(limit, resets)} />)
      ) : (
        <div className="quota-empty">
          <h2>Quota windows unavailable</h2>
          <p>QuotaStation will keep retrying the {provider} quota source.</p>
        </div>
      )}
      {/* Only a provider that grants reset credits has an inventory to report; for the
          others the row would say "Unknown" forever. */}
      {earnedResetCount === null ? null : (
        <div className="reset-inventory">
          <span>Earned resets</span>
          <strong>{earnedResetCount}</strong>
        </div>
      )}
      {!compact && resets.length > 0 ? <ResetHistory resets={resets} /> : null}
    </section>
  );
}
