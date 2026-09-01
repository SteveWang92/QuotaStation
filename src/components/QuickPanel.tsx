import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../errors";
import { formatCurrency, formatNumber } from "../format";
import { statusColor } from "../theme";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";
import { ProviderSetup } from "./ProviderSetup";
import { QuotaSection } from "./QuotaSection";

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
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const shell = useReportedHeight();
  // The panel is the quota glance, so a provider whose quota is switched off has no column
  // here at all, exactly as it has no panel on the dashboard and no slot in the widget.
  const providers = workspace.providers.filter((snapshot) => !snapshot.quotaDisabled);

  const refresh = useCallback(async () => {
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
    <main className="quick-panel-shell" ref={shell}>
      {/* Each column carries its own provider's status, and the aggregate here was the
          louder of those two said a second time. */}
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
          providers.map((snapshot) => (
            <ProviderColumn key={snapshot.provider} snapshot={snapshot} />
          ))
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
