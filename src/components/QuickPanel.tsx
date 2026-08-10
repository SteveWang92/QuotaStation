import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ArrowUpRight, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { formatCurrency, formatNumber } from "../format";
import type { ProviderSnapshot } from "../types";
import { QuotaSection } from "./QuotaSection";

export function QuickPanel({ initialSnapshot }: { initialSnapshot: ProviderSnapshot }) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [refreshing, setRefreshing] = useState(false);

  const loadSnapshot = useCallback(async () => {
    setSnapshot(await invoke<ProviderSnapshot>("get_snapshot"));
  }, []);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      setSnapshot(await invoke<ProviderSnapshot>("refresh_now"));
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let stop = () => {};
    void loadSnapshot();
    void listen<ProviderSnapshot>("snapshot-updated", ({ payload }) => setSnapshot(payload)).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    const timer = window.setInterval(() => void loadSnapshot(), 30_000);
    return () => {
      disposed = true;
      stop();
      window.clearInterval(timer);
    };
  }, [loadSnapshot]);

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
      <p className="quick-freshness">{snapshot.compactStatus.message}</p>
      <button type="button" className="dashboard-link" onClick={() => void invoke("open_dashboard")}>
        Open dashboard <ArrowUpRight aria-hidden="true" />
      </button>
    </main>
  );
}
