import { invoke } from "@tauri-apps/api/core";
import { ArrowUpRight, RefreshCw } from "lucide-react";
import { useCallback, useState } from "react";
import { errorMessage } from "../errors";
import { formatCurrency, formatNumber } from "../format";
import type { ProviderSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";
import { QuotaSection } from "./QuotaSection";

export function QuickPanel({ initialSnapshot }: { initialSnapshot: ProviderSnapshot }) {
  const { snapshot, error } = useSnapshot(initialSnapshot);
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
          <strong style={{ color: snapshot.compactStatus.color }}>{snapshot.compactStatus.label}</strong>
        </div>
        <button type="button" aria-label="Refresh quota and usage" onClick={() => void refresh()} disabled={refreshing}>
          <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
        </button>
      </header>
      <QuotaSection
        compact
        limits={snapshot.limits}
        earnedResetCount={snapshot.earnedResetCount}
        statusColor={snapshot.compactStatus.color}
      />
      <section className="quick-usage" aria-label="Today's usage">
        <div><span>Today</span><strong>{formatNumber(snapshot.today.total)}</strong><small>tokens</small></div>
        <div><span>API equivalent</span><strong>{formatCurrency(snapshot.apiEquivalentCostUsd)}</strong><small>estimated cost</small></div>
      </section>
      <p className={`quick-freshness${failure ? " failed" : ""}`}>{failure ?? snapshot.compactStatus.message}</p>
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
