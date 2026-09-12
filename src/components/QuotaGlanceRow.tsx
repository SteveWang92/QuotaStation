import { formatCompactCountdown, formatWindowBadge } from "../format";
import { quotaColor } from "../theme";
import type { LimitWindow, PaceLevel } from "../types";

/** What each pace is drawn as, and how it is read out. Nothing is drawn for `onTrack`. */
const PACE: Record<PaceLevel, { marker: string; label: string } | null> = {
  onTrack: null,
  ahead: { marker: "↑", label: "ahead of the window" },
  behind: { marker: "↓", label: "behind the window" },
};

/**
 * One quota window on one line: badge, bar, reading.
 *
 * Every cell is a fixed width and only the bar takes what is left, so two windows whose
 * countdowns are written with different numbers of characters are still drawn on the same
 * scale. The taskbar widget and the compact quick panel both render this; how wide those
 * cells are, and what the surface is drawn on, belongs to each surface's own stylesheet,
 * which is why nothing here carries a size.
 */
export function QuotaGlanceRow({
  limit,
  label,
  fallbackColor,
  title,
  showPace,
}: {
  limit: LimitWindow;
  /** Names this window where the row has no reading for a screen reader to read out. */
  label: string;
  /** What a row with no reading at all is drawn in, which is the provider's own status. */
  fallbackColor: string;
  /** The exact local reset time, for a surface with no room to print it beside the row. */
  title?: string;
  /**
   * Whether to mark a window being spent ahead of or behind its own clock. The taskbar
   * crops this row to the taskbar's height and has no character to spare for it, so the
   * surface says rather than the row assuming.
   */
  showPace?: boolean;
}) {
  const percent = limit.usedPercent === null ? null : `${Math.round(limit.usedPercent)}%`;
  const pace = showPace ? PACE[limit.pace] : null;
  const countdown = limit.resetsAt === null ? null : formatCompactCountdown(limit.resetsAt);
  return (
    <div className="glance-row" title={title}>
      <span className="glance-badge">
        {formatWindowBadge(limit.windowDurationMins, limit.label)}
      </span>
      {/* The bar fills with what has been consumed, exactly as the dashboard's meter does.
          Filling it with what is left would leave the same quota drawn one way here and the
          other way in the window above it. */}
      {limit.usedPercent === null ? (
        <i className="unknown" aria-hidden="true" />
      ) : (
        <i aria-hidden="true">
          <b style={{ width: `${limit.usedPercent}%`, background: quotaColor(limit) }} />
        </i>
      )}
      {/* Usage and reset share one cell so a window still waiting for its first reading
          shows a single dash on the same axis as the window below it, rather than two
          dashes pushed against the right edge. */}
      <span className="glance-reading">
        {percent === null && countdown === null ? (
          <em role="img" aria-label={`${label}: no reading yet`} style={{ color: fallbackColor }}>
            —
          </em>
        ) : (
          <>
            {percent !== null && (
              <em style={{ color: quotaColor(limit) }}>
                {percent}
                {pace ? (
                  <span role="img" aria-label={pace.label}>
                    {pace.marker}
                  </span>
                ) : null}
              </em>
            )}
            {percent !== null && countdown !== null && (
              <span className="glance-dot" aria-hidden="true">
                ·
              </span>
            )}
            {countdown !== null && limit.resetsAt !== null && (
              <time dateTime={new Date(limit.resetsAt * 1_000).toISOString()}>{countdown}</time>
            )}
          </>
        )}
      </span>
    </div>
  );
}
