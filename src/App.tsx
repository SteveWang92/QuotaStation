import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { QuotaSection } from "./components/QuotaSection";
import { QuickPanel } from "./components/QuickPanel";
import { StatusBar } from "./components/StatusBar";
import { UsageSummary } from "./components/UsageSummary";
import { createPresetRange, type DateRangeSelection } from "./dateRanges";
import type { DiagnosticsSnapshot, ProviderSnapshot, UsageRangeSnapshot } from "./types";

const EMPTY_SNAPSHOT: ProviderSnapshot = {
  provider: "codex",
  planType: null,
  limits: [],
  earnedResetCount: null,
  today: { input: 0, cacheRead: 0, output: 0, reasoning: 0, total: 0 },
  apiEquivalentCostUsd: null,
  models: [],
  freshness: "unavailable",
  staleAgeSeconds: null,
  compactStatus: {
    level: "unavailable",
    label: "Provider unavailable",
    message: "No current Codex quota data is available.",
    color: "#ff7469",
  },
  lastAttemptAt: null,
  lastSuccessAt: null,
  liveError: null,
  historyError: null,
  parserRevision: "033c1f7631f603fc939fdc85163e8203f0084f83",
  pricingCatalogRevision: "pending-build",
};

const INITIAL_RANGE = createPresetRange("today");
const EMPTY_USAGE_RANGE: UsageRangeSnapshot = {
  startDate: INITIAL_RANGE.startDate,
  endDate: INITIAL_RANGE.endDate,
  usage: { input: 0, cacheRead: 0, output: 0, reasoning: 0, total: 0 },
  apiEquivalentCostUsd: null,
  models: [],
  days: [],
};

const EMPTY_DIAGNOSTICS: DiagnosticsSnapshot = {
  watcher: { status: "starting", watchedLocationCount: 0, lastEventAt: null, error: null },
  acquisitions: [],
  retention: { status: "pending", lastCompletedAt: null, error: null },
  parserRevision: EMPTY_SNAPSHOT.parserRevision,
  pricingCatalogRevision: EMPTY_SNAPSHOT.pricingCatalogRevision,
};

const CURRENT_WINDOW_LABEL = getCurrentWindow().label;
document.documentElement.classList.toggle("quick-panel-window", CURRENT_WINDOW_LABEL === "quick-panel");

function Dashboard() {
  const [snapshot, setSnapshot] = useState<ProviderSnapshot>(EMPTY_SNAPSHOT);
  const [usageRange, setUsageRange] = useState<UsageRangeSnapshot>(EMPTY_USAGE_RANGE);
  const [activeRange, setActiveRange] = useState<DateRangeSelection>(INITIAL_RANGE);
  const [rangeLoading, setRangeLoading] = useState(false);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSnapshot>(EMPTY_DIAGNOSTICS);
  const activeRangeRef = useRef(INITIAL_RANGE);
  const rangeRequestId = useRef(0);

  const loadSnapshot = useCallback(async () => {
    const next = await invoke<ProviderSnapshot>("get_snapshot");
    setSnapshot(next);
  }, []);

  const loadUsageRange = useCallback(async (range: DateRangeSelection) => {
    const requestId = ++rangeRequestId.current;
    setRangeLoading(true);
    setRangeError(null);
    try {
      const next = await invoke<UsageRangeSnapshot>("get_usage_range", {
        startDate: range.startDate,
        endDate: range.endDate,
      });
      if (requestId === rangeRequestId.current) setUsageRange(next);
    } catch (error) {
      if (requestId === rangeRequestId.current) {
        setRangeError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (requestId === rangeRequestId.current) setRangeLoading(false);
    }
  }, []);

  const loadDiagnostics = useCallback(async () => {
    const next = await invoke<DiagnosticsSnapshot>("get_diagnostics");
    setDiagnostics(next);
  }, []);

  const selectRange = useCallback((range: DateRangeSelection) => {
    activeRangeRef.current = range;
    setActiveRange(range);
    void loadUsageRange(range);
  }, [loadUsageRange]);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const next = await invoke<ProviderSnapshot>("refresh_now");
      setSnapshot(next);
      await Promise.all([loadUsageRange(activeRangeRef.current), loadDiagnostics()]);
    } finally {
      setRefreshing(false);
    }
  }, [loadDiagnostics, loadUsageRange]);

  useEffect(() => {
    let disposed = false;
    let unlisten: Array<() => void> = [];
    void Promise.all([loadSnapshot(), loadUsageRange(activeRangeRef.current), loadDiagnostics()]);
    void Promise.all([
      listen<ProviderSnapshot>("snapshot-updated", ({ payload }) => {
        setSnapshot(payload);
        void loadDiagnostics();
      }),
      listen("history-updated", () => {
        void loadUsageRange(activeRangeRef.current);
      }),
    ]).then((listeners) => {
      if (disposed) listeners.forEach((stop) => stop());
      else unlisten = listeners;
    });
    const timer = window.setInterval(() => {
      void Promise.all([loadSnapshot(), loadUsageRange(activeRangeRef.current), loadDiagnostics()]);
    }, 30_000);
    return () => {
      disposed = true;
      unlisten.forEach((stop) => stop());
      window.clearInterval(timer);
    };
  }, [loadDiagnostics, loadSnapshot, loadUsageRange]);

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="identity">
          <h1>QuotaStation</h1>
          <span className="provider-name">Codex</span>
          <p>Local quota and usage data from the installed Codex client.</p>
        </div>
        <div className="header-actions">
          <button type="button" onClick={() => void refresh()} disabled={refreshing}>
            <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
            {refreshing ? "Refreshing" : "Refresh"}
          </button>
        </div>
      </header>
      <QuotaSection
        limits={snapshot.limits}
        earnedResetCount={snapshot.earnedResetCount}
        statusColor={snapshot.compactStatus.color}
      />
      <UsageSummary
        snapshot={snapshot}
        range={usageRange}
        selection={activeRange}
        loading={rangeLoading}
        error={rangeError}
        onSelectRange={selectRange}
      />
      <StatusBar snapshot={snapshot} diagnostics={diagnostics} />
    </main>
  );
}

export default function App() {
  return CURRENT_WINDOW_LABEL === "quick-panel"
    ? <QuickPanel initialSnapshot={EMPTY_SNAPSHOT} />
    : <Dashboard />;
}
