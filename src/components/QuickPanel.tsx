import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, RefreshCw } from "lucide-react";
import { useCallback, useState } from "react";
import { errorMessage } from "../errors";
import { formatCurrency, formatNumber } from "../format";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";
import { QuotaSection } from "./QuotaSection";

function ProviderColumn({ snapshot }: { snapshot: ProviderSnapshot }) {
  return (
    <section className="quick-provider" aria-label={`${snapshot.displayName} status`}>
      <header className="quick-provider-header">
        <h2>{snapshot.displayName}</h2>
        <span style={{ color: snapshot.compactStatus.color }}>{snapshot.compactStatus.label}</span>
      </header>
      <QuotaSection
        compact
        provider={snapshot.displayName}
        limits={snapshot.limits}
        earnedResetCount={snapshot.earnedResetCount}
        resets={snapshot.recentResets}
        statusColor={snapshot.compactStatus.color}
      />
      <section className="quick-usage" aria-label={`${snapshot.displayName} usage today`}>
        <div><span>Today</span><strong>{formatNumber(snapshot.today.total)}</strong><small>tokens</small></div>
        <div><span>API equivalent</span><strong>{formatCurrency(snapshot.apiEquivalentCostUsd)}</strong><small>estimated cost</small></div>
      </section>
      <p className="quick-freshness">{snapshot.compactStatus.message}</p>
    </section>
  );
}

export function QuickPanel({ initialWorkspace }: { initialWorkspace: WorkspaceSnapshot }) {
  const { workspace, error } = useSnapshot(initialWorkspace);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);

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
    <main className="quick-panel-shell">
      <header className="quick-panel-header">
        <div>
          <span>QuotaStation</span>
          <strong style={{ color: workspace.aggregate.color }}>{workspace.aggregate.label}</strong>
        </div>
        <button type="button" aria-label="Refresh quota and usage" onClick={() => void refresh()} disabled={refreshing}>
          <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
        </button>
      </header>
      <div className={`quick-providers${workspace.providers.length <= 1 ? " single" : ""}`}>
        {workspace.providers.map((snapshot) => (
          <ProviderColumn key={snapshot.provider} snapshot={snapshot} />
        ))}
      </div>
      {failure ? <p className="quick-freshness failed">{failure}</p> : null}
      <button
        type="button"
        className="dashboard-link"
        onClick={() => void invoke("open_dashboard").catch((cause) => setRefreshError(errorMessage(cause)))}
      >
        Open dashboard <ArrowUpRight aria-hidden="true" />
      </button>
    </main>
  );
}
