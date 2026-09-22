import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { logActivity } from "../activity";
import { useAppSettings } from "../appSettings";
import { bandGeometry } from "../charts";
import { toLocalDateString } from "../dateRanges";
import { errorMessage } from "../errors";
import {
  formatAxisDate,
  formatCompactNumber,
  formatCurrency,
  formatDelta,
  formatNumber,
  formatResetTimestamp,
  formatSeenOn,
  formatShortMoment,
  formatWindowBadge,
} from "../format";
import { statusColor } from "../theme";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";
import { ProviderSetup } from "./ProviderSetup";
import { QuotaGlanceRow } from "./QuotaGlanceRow";
import { QuotaSection } from "./QuotaSection";

/** The coordinate space the sparkline is drawn in; the element scales it to the column. */
const TREND_WIDTH = 100;
const TREND_HEIGHT = 20;

/** How many of the day's models a column this narrow can name with their shares. */
const COMPACT_MODELS = 2;

/**
 * A week of tokens as one row of columns, one per day and today at the right.
 *
 * The days are scaled against the busiest of the seven rather than against an axis: there is
 * no room for one here, and what a week is read for is its shape, which no axis changes. The
 * figures behind it are in the tooltip, and exactly on the dashboard.
 */
function CompactTrend({ snapshot }: { snapshot: ProviderSnapshot }) {
  const totals = snapshot.dailyTotals;
  const peak = Math.max(0, ...totals);
  // A week with nothing in it is a row of noughts, which says less than the line of usage
  // below it already does.
  if (peak === 0) return null;
  const { barWidth, left } = bandGeometry(TREND_WIDTH, totals.length, TREND_WIDTH);
  const midnight = new Date();
  midnight.setHours(0, 0, 0, 0);
  const dayOf = (index: number) => {
    const date = new Date(midnight);
    date.setDate(date.getDate() - (totals.length - 1 - index));
    return formatAxisDate(toLocalDateString(date));
  };
  return (
    <svg
      className="quick-trend"
      viewBox={`0 0 ${TREND_WIDTH} ${TREND_HEIGHT}`}
      preserveAspectRatio="none"
      role="img"
      aria-label={`${snapshot.displayName} tokens over the last seven days`}
    >
      <title>
        {totals.map((total, index) => `${dayOf(index)} ${formatCompactNumber(total)}`).join(" · ")}
      </title>
      {totals.map((total, index) => {
        // A day that was used at all keeps a visible sliver, so a quiet day and an unused
        // one are told apart rather than both reading as a gap.
        const height = total === 0 ? 0 : Math.max(1, (total / peak) * TREND_HEIGHT);
        return (
          <rect
            key={dayOf(index)}
            x={left(index)}
            y={TREND_HEIGHT - height}
            width={barWidth}
            height={height}
          />
        );
      })}
    </svg>
  );
}

/**
 * When this provider's quota last restarted and what it stood at.
 *
 * This is what explains a percentage that moved with nobody working: the window behind it
 * started again. The newest restart of any window is the one shown, because the panel has
 * room for one line and the newest is the one that explains the rows above it.
 */
function CompactRestart({ snapshot }: { snapshot: ProviderSnapshot }) {
  const restart = snapshot.recentResets[0];
  if (!restart) return null;
  const before = `${Math.round(restart.usedPercentBefore)}%`;
  const seenOn = formatSeenOn(restart);
  return (
    <p
      className="quick-restart"
      title={`${restart.windowLabel} restarted ${formatResetTimestamp(restart.anchoredAt)} at ${before} used${seenOn ? ` · ${seenOn}` : ""}`}
    >
      <span>Restart</span>
      <strong>{formatWindowBadge(restart.windowDurationMins, restart.windowLabel)}</strong>
      <time dateTime={new Date(restart.anchoredAt * 1_000).toISOString()}>
        {formatShortMoment(restart.anchoredAt)}
      </time>
      <span>· {before}</span>
    </p>
  );
}

/**
 * The same provider at compact density: one line per quota window, the week behind them, and
 * what today went to. The exact reset time, the tenths of a percent and the exact token count
 * are all a dashboard away.
 */
function CompactProvider({ snapshot }: { snapshot: ProviderSnapshot }) {
  const providerColor = statusColor(snapshot.compactStatus);
  // Yesterday and today are the last two days of the same week the trend is drawn from.
  const delta = formatDelta(snapshot.today.total, snapshot.dailyTotals.at(-2) ?? 0);
  return (
    <section className="quick-provider" aria-label={`${snapshot.displayName} status`}>
      <header className="quick-provider-header">
        <h2>{snapshot.displayName}</h2>
        <span style={{ color: providerColor }}>{snapshot.compactStatus.label}</span>
      </header>
      {snapshot.signInRequired ? (
        <p className="quick-provider-note">
          Signed out — sign in with the {snapshot.displayName} client again.
        </p>
      ) : snapshot.limits.length > 0 ? (
        <div className="quick-windows">
          {snapshot.limits.map((limit) => (
            <QuotaGlanceRow
              key={limit.kind}
              limit={limit}
              label={`${snapshot.displayName} ${limit.label}`}
              fallbackColor={providerColor}
              showPace
              // The row has no room for the exact local time, and it is the one thing here
              // that a countdown cannot be read off.
              title={`${limit.label} — ${formatResetTimestamp(limit.resetsAt)}`}
            />
          ))}
        </div>
      ) : (
        <p className="quick-provider-note">No quota window reported yet.</p>
      )}
      <CompactTrend snapshot={snapshot} />
      {/* Today's figures in their compact form: the exact count needs more width than the
          whole panel has, and the dashboard is where it is read exactly. */}
      <p className="quick-today">
        <span>Today</span>
        <strong>{formatCompactNumber(snapshot.today.total)}</strong>
        <span>· {formatCurrency(snapshot.apiEquivalentCostUsd)}</span>
        {delta ? <em title="Against the whole of yesterday">{delta}</em> : null}
      </p>
      <CompactRestart snapshot={snapshot} />
      {/* What today's tokens went to. A model name is the one thing here allowed to be cut
          short, because it is recognised from its front. */}
      {snapshot.models.slice(0, COMPACT_MODELS).map((model) => (
        <p
          className="quick-model"
          key={model.model}
          title={`${model.model} — ${formatNumber(model.tokens)} tokens today`}
        >
          <span>{model.model}</span>
          <em>{Math.round(model.percent)}%</em>
        </p>
      ))}
    </section>
  );
}

function ProviderColumn({ snapshot }: { snapshot: ProviderSnapshot }) {
  return (
    <section className="quick-provider" aria-label={`${snapshot.displayName} status`}>
      <header className="quick-provider-header">
        <h2>{snapshot.displayName}</h2>
        <span style={{ color: statusColor(snapshot.compactStatus) }}>
          {snapshot.compactStatus.label}
        </span>
      </header>
      {snapshot.signInRequired ? (
        <p className="quick-provider-note">
          Signed out — sign in with the {snapshot.displayName} client again.
        </p>
      ) : (
        <QuotaSection
          compact
          provider={snapshot.provider}
          providerName={snapshot.displayName}
          limits={snapshot.limits}
          earnedResetCount={snapshot.earnedResetCount}
          earnedResetExpiresAt={snapshot.earnedResetExpiresAt}
          resets={snapshot.recentResets}
        />
      )}
      <section className="quick-usage" aria-label={`${snapshot.displayName} usage today`}>
        <div>
          <span>Today</span>
          <strong>{formatNumber(snapshot.today.total)}</strong>
          <small>tokens</small>
        </div>
        <div>
          <span>API equivalent</span>
          <strong>{formatCurrency(snapshot.apiEquivalentCostUsd)}</strong>
          <small>estimated cost</small>
        </div>
      </section>
    </section>
  );
}

/**
 * The panel is a frameless window, so nothing trims it to its contents: without this it is
 * sized for the tallest case it might ever hold and everything shorter leaves dead space,
 * while everything taller scrolls. Reporting the rendered height lets the core grow the
 * window upwards from the tray instead, and the core clamps it to the work area — past
 * that the panel scrolls, because there is nowhere left to grow.
 */
function useReportedHeight() {
  const shell = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const element = shell.current;
    if (!element) return;
    let reported = 0;
    const report = () => {
      const height = Math.ceil(element.getBoundingClientRect().height);
      // Resizing the window re-runs the observer, so a report has to be worth making or
      // the two would trade single pixels back and forth forever.
      if (height <= 0 || Math.abs(height - reported) < 2) return;
      reported = height;
      void invoke("set_quick_panel_height", { height }).catch(() => {});
    };
    const observer = new ResizeObserver(report);
    observer.observe(element);
    report();
    return () => observer.disconnect();
  }, []);

  return shell;
}

export function QuickPanel({ initialWorkspace }: { initialWorkspace: WorkspaceSnapshot }) {
  const { workspace, error, loaded } = useSnapshot(initialWorkspace);
  const { settings } = useAppSettings();
  // The core sizes the window from the same setting, so the density the layout is drawn at
  // and the width it is given agree. Until the settings arrive there is nothing to draw
  // either way, and standard is what the core opened the window at.
  const compact = settings?.quickPanelDensity === "compact";
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const shell = useReportedHeight();
  // The panel is the quota glance, so a provider whose quota is switched off has no column
  // here at all, exactly as it has no panel on the dashboard and no slot in the widget.
  const providers = workspace.providers.filter((snapshot) => !snapshot.quotaDisabled);

  const refresh = useCallback(async () => {
    logActivity("refresh pressed in the quick panel");
    setRefreshing(true);
    setRefreshError(null);
    try {
      await invoke("refresh_now");
    } catch (cause) {
      setRefreshError(errorMessage(cause));
    } finally {
      setRefreshing(false);
    }
  }, []);

  const failure = refreshError ?? error;
  return (
    <main className={`quick-panel-shell${compact ? " compact" : ""}`} ref={shell}>
      {/* Each column carries its own provider's status, so the header repeats no aggregate. */}
      <header className="quick-panel-header">
        <strong>QuotaStation</strong>
        <button
          type="button"
          aria-label="Refresh quota and usage"
          onClick={() => void refresh()}
          disabled={refreshing}
        >
          <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
        </button>
      </header>
      <div className={`quick-providers${providers.length <= 1 ? " single" : ""}`}>
        {providers.length > 0 ? (
          providers.map((snapshot) =>
            compact ? (
              <CompactProvider key={snapshot.provider} snapshot={snapshot} />
            ) : (
              <ProviderColumn key={snapshot.provider} snapshot={snapshot} />
            ),
          )
        ) : !loaded ? null : workspace.providers.length > 0 ? (
          <p className="quick-provider-note">Quota tracking is off for every provider.</p>
        ) : (
          <ProviderSetup compact />
        )}
      </div>
      {failure ? <p className="quick-freshness failed">{failure}</p> : null}
      <button
        type="button"
        className="dashboard-link"
        onClick={() =>
          void invoke("open_dashboard").catch((cause) => setRefreshError(errorMessage(cause)))
        }
      >
        Open dashboard <ArrowUpRight aria-hidden="true" />
      </button>
    </main>
  );
}
