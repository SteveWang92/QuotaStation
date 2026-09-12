import {
  formatCurrency,
  formatDayAndTime,
  formatDelta,
  formatDuration,
  formatNumber,
} from "../format";
import type { SessionCost, SessionCostSnapshot } from "../types";

/**
 * How far apart the two figures have to be before the row says so. Under it the catalog
 * and the vendor's own accounting agree as closely as two estimates of the same work ever
 * do, and marking every row would say nothing about the one that has drifted.
 */
const WIDE_GAP_PERCENT = 10;

/** How much of the client's session identifier is enough to recognise it by. */
const SESSION_ID_LENGTH = 8;

interface SessionTableProps {
  snapshot: SessionCostSnapshot | null;
  /** What the table covers, in the words of the range control above it. */
  rangeLabel: string;
  /** Anything true of this reading that the rows cannot say for themselves. */
  notes: string[];
  loading: boolean;
}

/**
 * Every session in the range, priced twice.
 *
 * Neither column is a bill: a subscription charges nothing per token, so the pair says
 * whether this machine's pricing catalog still tracks what the provider's own client
 * charged the same session to.
 */
export function SessionTable({ snapshot, rangeLabel, notes, loading }: SessionTableProps) {
  const sessions = snapshot?.sessions ?? [];
  const gap =
    snapshot === null ? null : formatDelta(snapshot.computedCostUsd, snapshot.reportedCostUsd);

  return (
    <article className="history-card session-card">
      <div className="card-heading">
        <div>
          <h3>Sessions</h3>
          <span>
            {rangeLabel} · {sessions.length} session{sessions.length === 1 ? "" : "s"}
          </span>
        </div>
        <span>What the client charged against what the catalog makes of it</span>
      </div>

      {notes.map((note) => (
        <p className="session-note" key={note}>
          {note}
        </p>
      ))}

      <div className="summary-strip">
        <SessionTotal label="Reported by the client" value={snapshot?.reportedCostUsd ?? null} />
        <SessionTotal label="Priced from the catalog" value={snapshot?.computedCostUsd ?? null} />
        <div>
          <span>Estimate against the client</span>
          <strong>{gap ?? "—"}</strong>
        </div>
      </div>

      {sessions.length === 0 ? (
        <p className="empty-copy">
          No session comparisons in this range. Only sessions whose client recorded a cost of its
          own can be compared, and comparisons are kept for {snapshot?.retentionDays ?? 90} days.
        </p>
      ) : (
        <div
          className={`session-table${loading ? " loading" : ""}`}
          role="table"
          aria-label="Per-session cost comparison"
        >
          <div className="session-table-row session-table-header" role="row">
            <span role="columnheader">Session</span>
            <span role="columnheader">Started</span>
            <span role="columnheader">Ran for</span>
            <span role="columnheader">Models</span>
            <span role="columnheader">Tokens</span>
            <span role="columnheader">Reported</span>
            <span role="columnheader">Computed</span>
            <span role="columnheader">Gap</span>
            <span role="columnheader">Lines</span>
            <span role="columnheader">Notes</span>
          </div>
          {sessions.map((session) => (
            <SessionRow key={session.sessionId} session={session} />
          ))}
        </div>
      )}
    </article>
  );
}

function SessionRow({ session }: { session: SessionCost }) {
  const gap = formatDelta(session.computedCostUsd, session.reportedCostUsd);
  const wide =
    gap !== null &&
    Math.abs(
      ((session.computedCostUsd - session.reportedCostUsd) / session.reportedCostUsd) * 100,
    ) >= WIDE_GAP_PERCENT;
  const [first, ...rest] = session.models;

  return (
    <div className="session-table-row" role="row">
      <strong role="cell" title={session.sessionId}>
        {session.sessionId.slice(0, SESSION_ID_LENGTH)}
      </strong>
      <span role="cell">{formatDayAndTime(session.sessionStartedAt)}</span>
      <span role="cell" title={`${formatDuration(session.apiDurationMs)} waiting on the provider`}>
        {formatDuration(session.totalDurationMs)}
      </span>
      <span role="cell" title={session.models.join(", ")}>
        {first ?? "—"}
        {rest.length > 0 ? ` +${rest.length}` : ""}
      </span>
      <span role="cell">{formatNumber(session.usage.total)}</span>
      <span role="cell">{formatCurrency(session.reportedCostUsd)}</span>
      <span role="cell">{formatCurrency(session.computedCostUsd)}</span>
      {/* Neither direction is better than the other — both figures are estimates of the
          same work — so only the width of the gap is marked, and only once it is wide
          enough to mean the catalog has drifted from what the client charges. */}
      <span role="cell" className={wide ? "session-gap-wide" : ""}>
        {gap ?? "—"}
      </span>
      <span role="cell">
        +{formatNumber(session.linesAdded)} −{formatNumber(session.linesRemoved)}
      </span>
      <span role="cell" className="session-flags">
        {session.independent ? null : (
          <em title="The client's own per-message costs priced this session, so both figures are the same number.">
            Same source
          </em>
        )}
        {session.reportedComplete ? null : (
          <em title="The client met a model it has no price for, so its total is short of the session.">
            Client short
          </em>
        )}
      </span>
    </div>
  );
}

function SessionTotal({ label, value }: { label: string; value: number | null }) {
  return (
    <div>
      <span>{label}</span>
      {/* Nothing has arrived yet on the first read, which is not the same as nothing to
          report; the figure stays blank rather than claiming a nought was spent. */}
      <strong>{value === null ? "—" : formatCurrency(value)}</strong>
    </div>
  );
}
