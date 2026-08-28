import { Check } from "lucide-react";
import { useCallback } from "react";
import { saveAppSettings, useAppSettings } from "../appSettings";
import {
  formatCompactNumber,
  formatCountdown,
  formatEarlyBy,
  formatResetTimestamp,
} from "../format";
import { quotaColor } from "../theme";
import type { LimitResetEvent, LimitWindow, ProviderKey } from "../types";

interface QuotaSectionProps {
  /** Which provider these windows belong to, which is half of a dismissed note's key. */
  provider: ProviderKey;
  /** Display name of the provider these windows belong to, for labels and empty copy. */
  providerName: string;
  limits: LimitWindow[];
  earnedResetCount: number | null;
  earnedResetExpiresAt: number | null;
  resets: LimitResetEvent[];
  /**
   * The notice keys of every window on display right now, so dismissing one can rewrite
   * the record against them rather than appending to it forever. Surfaces that show no
   * dismiss control — the quick panel — leave this out.
   */
  liveWindowKeys?: string[];
  compact?: boolean;
}

/**
 * How one early-restart note is recorded once it has been read.
 *
 * The note explains the expiry the window is showing, so keying it on that expiry is what
 * brings the note back at the next restart and never brings back the one already read.
 */
export function resetNoticeKey(
  provider: ProviderKey,
  windowKind: LimitWindow["kind"],
  resetsAt: number | null,
): string {
  return `${provider}:${windowKind}:${resetsAt ?? "unknown"}`;
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
      event.windowDurationMins === limit.windowDurationMins,
  );
}

function QuotaRow({
  limit,
  origin,
  onDismissOrigin,
}: {
  limit: LimitWindow;
  origin?: LimitResetEvent;
  /** Absent where the surface has no room to acknowledge the note, such as the quick panel. */
  onDismissOrigin?: () => void;
}) {
  const used = limit.usedPercent;
  return (
    // Each window is coloured by its own reading: the provider's status is the loudest of
    // them, and inheriting it would paint an untouched window in the colour of a spent one.
    <div
      className={`quota-row${limit.freshness === "stale" ? " stale" : ""}`}
      style={{ "--quota-status-color": quotaColor(limit) } as React.CSSProperties}
    >
      {/* The label already names the duration, and which source produced the reading is a
          diagnostic rather than something to read at a glance, so it lives in the settings
          page beside the acquisition paths it belongs to. */}
      <div className="quota-label">
        <h2>{limit.label}</h2>
      </div>
      {/* The bar repeats the share that is written out beside it, so it is hidden from a
          screen reader rather than labelled twice. */}
      <div className="quota-meter" aria-hidden="true">
        <div className="quota-track">
          {used === null ? <span className="unknown" /> : <span style={{ width: `${used}%` }} />}
        </div>
      </div>
      <div className="quota-percent">
        <strong>{used === null ? "—" : `${used.toFixed(1)}%`}</strong>
        <span>{used === null ? "unavailable" : "used"}</span>
      </div>
      <div className="quota-reset">
        <span>Resets in</span>
        <strong>{formatCountdown(limit.resetsAt)}</strong>
        <time
          dateTime={
            limit.resetsAt === null ? undefined : new Date(limit.resetsAt * 1000).toISOString()
          }
        >
          {formatResetTimestamp(limit.resetsAt)}
        </time>
      </div>
      {origin ? (
        <p className="quota-origin">
          <span>
            Possibly restarted early on {formatResetTimestamp(origin.anchoredAt)} — estimated{" "}
            {formatEarlyBy(origin.earlyBySeconds)}, after a {origin.usedPercentBefore.toFixed(0)}%
            usage reading.
          </span>
          {/* The note stands for as long as this window does, which is days. Acknowledging
              it is the only way it ever goes away, and the next restart brings it back. */}
          {onDismissOrigin ? (
            <button type="button" onClick={onDismissOrigin}>
              <Check aria-hidden="true" /> Got it
            </button>
          ) : null}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The last restart recorded of each window, which is the one that explains the window
 * running now. Everything before it is history rather than status, and the settings page
 * lists that in full.
 *
 * A window is told apart by how long it runs, not by the slot it arrived in: Codex moves
 * its windows between `primary` and `secondary`, and grouping by slot puts a weekly
 * restart from before such a move on the row that now belongs to the five-hour window.
 */
function latestPerWindow(resets: LimitResetEvent[]): LimitResetEvent[] {
  const seen = new Set<number>();
  return resets.filter((event) => {
    if (seen.has(event.windowDurationMins)) return false;
    seen.add(event.windowDurationMins);
    return true;
  });
}

function LatestResets({ resets }: { resets: LimitResetEvent[] }) {
  return (
    <ul className="latest-resets">
      {resets.map((event) => (
        <li key={event.windowDurationMins} className={event.classification}>
          <span>{event.windowLabel} last restarted</span>
          <time dateTime={new Date(event.anchoredAt * 1000).toISOString()}>
            {formatResetTimestamp(event.anchoredAt)}
          </time>
          <strong>{event.usedPercentBefore.toFixed(0)}% used</strong>
          {/* Usage is stored by the hour, so a window's total is exact in the middle and
              approximate at its two ends; the tilde is what says so at a glance. */}
          <strong
            title={
              event.tokensInWindow === null
                ? "No hourly usage was recorded for this window"
                : "Summed from the hours that began inside this window"
            }
          >
            {event.tokensInWindow === null ? "—" : `~${formatCompactNumber(event.tokensInWindow)}`}
          </strong>
          <em>
            {event.classification === "unplanned"
              ? `possibly ${formatEarlyBy(event.earlyBySeconds)}`
              : "appears on schedule"}
          </em>
        </li>
      ))}
    </ul>
  );
}

export function QuotaSection({
  provider,
  providerName,
  limits,
  earnedResetCount,
  earnedResetExpiresAt,
  resets,
  liveWindowKeys,
  compact = false,
}: QuotaSectionProps) {
  const settings = useAppSettings();
  const dismissed = settings?.dismissedResetNotices ?? [];

  const dismiss = useCallback(
    async (key: string) => {
      // Only the notes for windows still on display are carried forward, so the record
      // stays as short as the number of windows rather than growing with every restart.
      await saveAppSettings((saved) => {
        const kept = (liveWindowKeys ?? []).filter(
          (live) => live !== key && saved.dismissedResetNotices.includes(live),
        );
        return { dismissedResetNotices: [...kept, key] };
      });
    },
    [liveWindowKeys],
  );

  const latest = latestPerWindow(resets);
  return (
    <section
      className={`quota-section${compact ? " compact" : ""}`}
      aria-label={`${providerName} quota windows`}
    >
      {limits.length > 0 ? (
        limits.map((limit) => {
          const key = resetNoticeKey(provider, limit.kind, limit.resetsAt);
          const origin = dismissed.includes(key) ? undefined : originOf(limit, resets);
          return (
            <QuotaRow
              key={limit.kind}
              limit={limit}
              origin={origin}
              onDismissOrigin={liveWindowKeys === undefined ? undefined : () => void dismiss(key)}
            />
          );
        })
      ) : (
        <div className="quota-empty">
          <h2>Quota windows unavailable</h2>
          <p>QuotaStation will keep retrying the {providerName} quota source.</p>
        </div>
      )}
      {/* Only a provider that grants reset credits has an inventory to report; for the
          others the row would say "Unknown" forever. */}
      {earnedResetCount === null ? null : (
        <div className="reset-inventory">
          <div className="reset-count">
            <span>Earned resets</span>
            <strong>{earnedResetCount}</strong>
          </div>
          {/* A credit that never expires publishes no deadline, and a provider that sends
              only the count publishes none either. Both are silence rather than "never". */}
          {earnedResetExpiresAt === null || earnedResetCount === 0 ? null : (
            <span className="reset-expiry">
              <span className="reset-expiry-label">First expires in</span>
              <strong>{formatCountdown(earnedResetExpiresAt)}</strong>
              <time dateTime={new Date(earnedResetExpiresAt * 1000).toISOString()}>
                {formatResetTimestamp(earnedResetExpiresAt)}
              </time>
            </span>
          )}
        </div>
      )}
      {!compact && latest.length > 0 ? <LatestResets resets={latest} /> : null}
    </section>
  );
}
