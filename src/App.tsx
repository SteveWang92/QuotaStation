import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { QuotaSection } from "./components/QuotaSection";
import { StatusBar } from "./components/StatusBar";
import { UsageSummary } from "./components/UsageSummary";
import { formatTimestamp } from "./format";
import type { ProviderSnapshot } from "./types";

const EMPTY_SNAPSHOT: ProviderSnapshot = {
  provider: "codex",
  planType: null,
  limits: [],
  earnedResetCount: null,
  today: { input: 0, cacheRead: 0, output: 0, reasoning: 0, total: 0 },
  apiEquivalentCostUsd: null,
  models: [],
  freshness: "unavailable",
  lastAttemptAt: null,
  lastSuccessAt: null,
  liveError: null,
  historyError: null,
  parserRevision: "033c1f7631f603fc939fdc85163e8203f0084f83",
  pricingCatalogRevision: "pending-build",
};

export default function App() {
  const [snapshot, setSnapshot] = useState<ProviderSnapshot>(EMPTY_SNAPSHOT);
  const [refreshing, setRefreshing] = useState(false);

  const loadSnapshot = useCallback(async () => {
    const next = await invoke<ProviderSnapshot>("get_snapshot");
    setSnapshot(next);
  }, []);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const next = await invoke<ProviderSnapshot>("refresh_now");
      setSnapshot(next);
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void loadSnapshot();
    const timer = window.setInterval(() => void loadSnapshot(), 30_000);
    return () => window.clearInterval(timer);
  }, [loadSnapshot]);

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="identity">
          <h1>QuotaStation</h1>
          <span className="provider-name">Codex</span>
          <p>Local quota and usage data from the installed Codex client.</p>
        </div>
        <div className="header-actions">
          <span>Last updated <strong>{formatTimestamp(snapshot.lastAttemptAt)}</strong></span>
          <button type="button" onClick={() => void refresh()} disabled={refreshing}>
            <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
            {refreshing ? "Refreshing" : "Refresh"}
          </button>
        </div>
      </header>
      <QuotaSection limits={snapshot.limits} earnedResetCount={snapshot.earnedResetCount} />
      <UsageSummary snapshot={snapshot} />
      <StatusBar snapshot={snapshot} />
    </main>
  );
}
