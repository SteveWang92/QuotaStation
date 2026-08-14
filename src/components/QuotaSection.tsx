import type { LimitResetEvent, LimitWindow } from "../types";
import { formatCountdown, formatEarlyBy, formatResetTimestamp, formatWindowDuration } from "../format";

interface QuotaSectionProps {
  /** Display name of the provider these windows belong to, for labels and empty copy. */
  provider: string;
  limits: LimitWindow[];
  earnedResetCount: number | null;
  resets: LimitResetEvent[];
  statusColor: string;
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
  const remaining = limit.remainingPercent;
  return (
    <div className={`quota-row${limit.freshness === "stale" ? " stale" : ""}`}>
      <div className="quota-label">
        <h2>{limit.label}</h2>
        <p>{formatWindowDuration(limit.windowDurationMins)}</p>
        <small>
          {limit.source === "app_server"
            ? "App server"
            : limit.source === "status_line"
              ? "Status line"
              : "Session log"}{" "}
          · as of {formatResetTimestamp(limit.observedAt)}
        </small>
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
        <strong>{remaining === null ? "—" : `${remaining.toFixed(1)}%`}</strong>
        <span>{remaining === null ? "unavailable" : "remaining"}</span>
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

export function QuotaSection({ provider, limits, earnedResetCount, resets, statusColor, compact = false }: QuotaSectionProps) {
  return (
    <section
      className={`quota-section${compact ? " compact" : ""}`}
      aria-label={`${provider} quota windows`}
      style={{ "--quota-status-color": statusColor } as React.CSSProperties}
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
