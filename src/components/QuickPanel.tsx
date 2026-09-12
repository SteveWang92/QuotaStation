import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { logActivity } from "../activity";
import { useAppSettings } from "../appSettings";
import { errorMessage } from "../errors";
import { formatCompactNumber, formatCurrency, formatNumber, formatResetTimestamp } from "../format";
import { statusColor } from "../theme";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";
import { ProviderSetup } from "./ProviderSetup";
import { QuotaGlanceRow } from "./QuotaGlanceRow";
import { QuotaSection } from "./QuotaSection";

/**
 * The same provider at compact density: one line per quota window and one line of usage,
 * because the panel is one narrow column and the exact reset time, the tenths of a percent
 * and the exact token count are all a dashboard away.
 */
function CompactProvider({ snapshot }: { snapshot: ProviderSnapshot }) {
  const providerColor = statusColor(snapshot.compactStatus);
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
              // The row has no room for the exact local time, and it is the one thing here
              // that a countdown cannot be read off.
              title={`${limit.label} — ${formatResetTimestamp(limit.resetsAt)}`}
            />
          ))}
        </div>
      ) : (
        <p className="quick-provider-note">No quota window reported yet.</p>
      )}
      {/* Today's figures in their compact form: the exact count needs more width than the
          whole panel has, and the dashboard is where it is read exactly. */}
      <p className="quick-today">
        <span>Today</span>
        <strong>{formatCompactNumber(snapshot.today.total)}</strong>
        <span>· {formatCurrency(snapshot.apiEquivalentCostUsd)}</span>
      </p>
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
