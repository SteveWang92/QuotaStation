import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "./errors";
import { useSnapshot } from "./useSnapshot";
import { ClaudeCrossCheck } from "./components/ClaudeCrossCheck";
import { ClaudeStatusLine } from "./components/ClaudeStatusLine";
import { ProviderSetup } from "./components/ProviderSetup";
import { QuotaSection } from "./components/QuotaSection";
import { QuickPanel } from "./components/QuickPanel";
import { TaskbarWidget } from "./components/TaskbarWidget";
import { StatusBar } from "./components/StatusBar";
import { UsageSummary } from "./components/UsageSummary";
import { createPresetRange, type DateRangeSelection } from "./dateRanges";
import type {
  DiagnosticsSnapshot,
  ProviderKey,
  UsageRangeSnapshot,
  WorkspaceSnapshot,
} from "./types";
import { EMPTY_WORKSPACE, resolveProviderKey } from "./workspace";

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
  parserRevision: "",
  pricingCatalogRevision: "",
};

const CURRENT_WINDOW_LABEL = getCurrentWindow().label;
document.documentElement.classList.toggle("compact-window", CURRENT_WINDOW_LABEL !== "main");
document.documentElement.classList.toggle("taskbar-window", CURRENT_WINDOW_LABEL === "taskbar-widget");

function Dashboard() {
  const [usageRange, setUsageRange] = useState<UsageRangeSnapshot>(EMPTY_USAGE_RANGE);
  const [activeRange, setActiveRange] = useState<DateRangeSelection>(INITIAL_RANGE);
  const [rangeLoading, setRangeLoading] = useState(false);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSnapshot>(EMPTY_DIAGNOSTICS);
  const activeRangeRef = useRef(INITIAL_RANGE);
  // The usage history is long, so it shows one provider at a time while the quota
  // sections above show them all.
  const [selectedProvider, setSelectedProvider] = useState<ProviderKey>("codex");
  const providerRef = useRef<ProviderKey>("codex");
  const rangeRequestId = useRef(0);
  const rangeRequested = useRef(false);

  const loadUsageRange = useCallback(async (range: DateRangeSelection, rangeProvider: ProviderKey) => {
    const requestId = ++rangeRequestId.current;
    setRangeLoading(true);
    setRangeError(null);
    try {
      const next = await invoke<UsageRangeSnapshot>("get_usage_range", {
        provider: rangeProvider,
        startDate: range.startDate,
        endDate: range.endDate,
      });
      if (requestId === rangeRequestId.current) setUsageRange(next);
    } catch (error) {
      if (requestId === rangeRequestId.current) setRangeError(errorMessage(error));
    } finally {
      if (requestId === rangeRequestId.current) setRangeLoading(false);
    }
  }, []);

  const loadDiagnostics = useCallback(async () => {
    try {
      setDiagnostics(await invoke<DiagnosticsSnapshot>("get_diagnostics"));
      setCommandError(null);
    } catch (error) {
      setCommandError(errorMessage(error));
    }
  }, []);

  // The shared subscription retries until the core is ready, so the first usage
  // range read waits for it instead of failing against an unmanaged state.
  const onSnapshot = useCallback((nextWorkspace: WorkspaceSnapshot) => {
    void loadDiagnostics();
    const rangeProvider = resolveProviderKey(nextWorkspace.providers, providerRef.current);
    if (!rangeProvider) return;
    const providerChanged = rangeProvider !== providerRef.current;
    if (providerChanged) {
      providerRef.current = rangeProvider;
      setSelectedProvider(rangeProvider);
    }
    if (!rangeRequested.current || providerChanged) {
      rangeRequested.current = true;
      void loadUsageRange(activeRangeRef.current, rangeProvider);
    }
  }, [loadDiagnostics, loadUsageRange]);

  const { workspace, error: snapshotError, loaded } = useSnapshot(EMPTY_WORKSPACE, onSnapshot);
  const historyProvider =
    workspace.providers.find((provider) => provider.provider === selectedProvider) ??
    workspace.providers[0];

  const selectProvider = useCallback(
    (provider: ProviderKey) => {
      providerRef.current = provider;
      setSelectedProvider(provider);
      void loadUsageRange(activeRangeRef.current, provider);
    },
    [loadUsageRange],
  );

  const selectRange = useCallback((range: DateRangeSelection) => {
    activeRangeRef.current = range;
    setActiveRange(range);
    void loadUsageRange(range, providerRef.current);
  }, [loadUsageRange]);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      // refresh_now publishes the new snapshot through the shared subscription.
      await invoke("refresh_now");
      setCommandError(null);
      await Promise.all([loadUsageRange(activeRangeRef.current, providerRef.current), loadDiagnostics()]);
    } catch (error) {
      setCommandError(errorMessage(error));
    } finally {
      setRefreshing(false);
    }
  }, [loadDiagnostics, loadUsageRange]);

  useEffect(() => {
    let disposed = false;
    let stopListening = () => {};
    void listen("history-updated", () => {
      rangeRequested.current = true;
      void loadUsageRange(activeRangeRef.current, providerRef.current);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stopListening = unlisten;
      })
      .catch((error) => {
        if (!disposed) setCommandError(errorMessage(error));
      });
    const timer = window.setInterval(() => {
      if (rangeRequested.current) void loadUsageRange(activeRangeRef.current, providerRef.current);
    }, 30_000);
    return () => {
      disposed = true;
      stopListening();
      window.clearInterval(timer);
    };
  }, [loadUsageRange]);

  const showClaudeCards = workspace.providers.some((provider) => provider.provider === "claude");

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="identity">
          <h1>QuotaStation</h1>
          <span className="provider-name">
            {workspace.providers.map((provider) => provider.displayName).join(" · ")}
          </span>
          <p>Local quota and usage data from the installed provider clients.</p>
        </div>
        <div className="header-actions">
          <button type="button" onClick={() => void refresh()} disabled={refreshing}>
            <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
            {refreshing ? "Refreshing" : "Refresh"}
          </button>
        </div>
      </header>
      {showClaudeCards ? <ClaudeStatusLine /> : null}
      {showClaudeCards ? <ClaudeCrossCheck /> : null}
      {loaded && workspace.providers.length === 0 ? <ProviderSetup /> : null}
      <div className={`provider-grid${workspace.providers.length <= 1 ? " single" : ""}`}>
        {workspace.providers.map((provider) => (
          <section key={provider.provider} className="provider-panel">
            <header className="provider-panel-header">
              <h2>{provider.displayName}</h2>
              <span style={{ color: provider.compactStatus.color }}>{provider.compactStatus.label}</span>
            </header>
            <QuotaSection
              provider={provider.displayName}
              limits={provider.limits}
              earnedResetCount={provider.earnedResetCount}
              resets={provider.recentResets}
              statusColor={provider.compactStatus.color}
            />
            {/* A second source that was deliberately switched on and then answered nothing
                has to say so here; the provider itself is still reporting. */}
            {provider.crossCheckError ? (
              <p className="provider-note">{provider.crossCheckError}</p>
            ) : null}
          </section>
        ))}
      </div>
      {historyProvider ? (
        <UsageSummary
          snapshot={historyProvider}
          providers={workspace.providers}
          activeProvider={selectedProvider}
          onSelectProvider={selectProvider}
          range={usageRange}
          selection={activeRange}
          loading={rangeLoading}
          error={rangeError}
          onSelectRange={selectRange}
        />
      ) : null}
      <StatusBar
        status={workspace.aggregate}
        diagnostics={diagnostics}
        interfaceError={snapshotError ?? commandError}
      />
    </main>
  );
}

export default function App() {
  if (CURRENT_WINDOW_LABEL === "quick-panel") return <QuickPanel initialWorkspace={EMPTY_WORKSPACE} />;
  if (CURRENT_WINDOW_LABEL === "taskbar-widget") return <TaskbarWidget initialWorkspace={EMPTY_WORKSPACE} />;
  return <Dashboard />;
}
