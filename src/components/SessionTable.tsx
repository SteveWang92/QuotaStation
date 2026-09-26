import { useState } from "react";
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

/**
 * Which sessions the table lists. Every session is worth reading — what it cost, how long
 * it ran, what it changed — and only some of them can be checked against the client's own
 * accounting, so the comparison is a filter over the list rather than the list itself.
 */
type SessionFilter = "all" | "compared";

interface SessionTableProps {
  snapshot: SessionCostSnapshot | null;
  /** What the table covers, in the words of the range control above it. */
  rangeLabel: string;
  /** Anything true of this reading that the rows cannot say for themselves. */
  notes: string[];
  loading: boolean;
}

/**
 * Every session in the range, priced from the catalog, and priced again by the client
 * where the client keeps a figure of its own.
 *
 * Neither column is a bill: a subscription charges nothing per token, so the pair says
 * whether this machine's pricing catalog still tracks what the provider's own client
 * charged the same session to.
 */
export function SessionTable({ snapshot, rangeLabel, notes, loading }: SessionTableProps) {
  const [filter, setFilter] = useState<SessionFilter>("all");
  const sessions = snapshot?.sessions ?? [];
  const compared = sessions.filter((session) => session.reportedCostUsd !== null);
  const shown = filter === "compared" ? compared : sessions;
  const gap =
    snapshot === null ? null : formatDelta(snapshot.computedCostUsd, snapshot.reportedCostUsd);

  return (
    <article className="history-card session-card">
      <div className="card-heading">
        <div>
          <h3>Sessions</h3>
          <span>
            {rangeLabel} · {shown.length} session{shown.length === 1 ? "" : "s"}
            {filter === "all" && sessions.length > 0
              ? ` · ${compared.length} priced by the client too`
              : ""}
          </span>
        </div>
        <div className="view-tabs" role="tablist" aria-label="Which sessions to list">
          <button
            type="button"
            role="tab"
            aria-selected={filter === "all"}
            className={filter === "all" ? "active" : ""}
            onClick={() => setFilter("all")}
          >
            All
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={filter === "compared"}
            className={filter === "compared" ? "active" : ""}
            onClick={() => setFilter("compared")}
          >
            Compared
          </button>
        </div>
      </div>

      {notes.map((note) => (
        <p className="session-note" key={note}>
          {note}
        </p>
      ))}

      {/* The three figures describe the comparable sessions whichever list is on screen:
          a total that added every computed cost to a reported one covering a few of them
          would state a gap that measures which sessions carry the record. */}
      <div className="summary-strip">
        <SessionTotal label="Reported by the client" value={snapshot?.reportedCostUsd ?? null} />
        <SessionTotal label="Priced from the catalog" value={snapshot?.computedCostUsd ?? null} />
        <div>
          <span>Estimate against the client</span>
          <strong>{gap ?? "—"}</strong>
        </div>
      </div>

      {shown.length === 0 ? (
        <p className="empty-copy">
          {filter === "compared" && sessions.length > 0
            ? "No session in this range carries the client's own cost. Claude Code began recording it partway through its life, and Codex records none at all."
            : "No sessions in this range. A session is listed once QuotaStation has read its log."}
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
          {shown.map((session) => (
            <SessionRow key={session.sessionId} session={session} />
          ))}
        </div>
      )}
    </article>
  );
}

function SessionRow({ session }: { session: SessionCost }) {
  const reported = session.reportedCostUsd;
  const gap = reported === null ? null : formatDelta(session.computedCostUsd, reported);
  const wide =
    reported !== null &&
    gap !== null &&
    Math.abs(((session.computedCostUsd - reported) / reported) * 100) >= WIDE_GAP_PERCENT;
  const [first, ...rest] = session.models;

  return (
    <div className="session-table-row" role="row">
      <strong role="cell" title={session.sessionId}>
        {session.sessionId.slice(0, SESSION_ID_LENGTH)}
      </strong>
      <span role="cell">{formatDayAndTime(session.sessionStartedAt)}</span>
      <span
        role="cell"
        title={
          session.apiDurationMs === null
            ? undefined
            : `${formatDuration(session.apiDurationMs)} waiting on the provider`
        }
      >
        {formatDuration(session.durationMs)}
      </span>
      <span role="cell" title={session.models.join(", ")}>
        {first ?? "—"}
        {rest.length > 0 ? ` +${rest.length}` : ""}
      </span>
      <span role="cell">{formatNumber(session.usage.total)}</span>
      {/* A client that priced nothing is not a nought: the column stays empty rather than
          claiming the session was free. */}
      <span role="cell">{reported === null ? "—" : formatCurrency(reported)}</span>
      <span role="cell">{formatCurrency(session.computedCostUsd)}</span>
      {/* Neither direction is better than the other — both figures are estimates of the
          same work — so only the width of the gap is marked, and only once it is wide
          enough to mean the catalog has drifted from what the client charges. */}
      <span role="cell" className={wide ? "session-gap-wide" : ""}>
        {gap ?? "—"}
      </span>
      <span role="cell">
        {session.linesAdded === null || session.linesRemoved === null
          ? "—"
          : `+${formatNumber(session.linesAdded)} −${formatNumber(session.linesRemoved)}`}
      </span>
      <span role="cell" className="session-flags">
        {session.reportedCostUsd === null || session.independent ? null : (
          <em title="The client's own per-message costs priced this session, so both figures are the same number.">
            Same source
          </em>
        )}
        {session.reportedComplete === false ? (
          <em title="The client met a model it has no price for, so its total is short of the session.">
            Client short
          </em>
        ) : null}
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
