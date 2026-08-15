import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "./errors";
import { useSnapshot } from "./useSnapshot";
import { ProviderSetup } from "./components/ProviderSetup";
import { SettingsDialog } from "./components/SettingsDialog";
import { QuotaSection } from "./components/QuotaSection";
import { QuickPanel } from "./components/QuickPanel";
import { TaskbarWidget } from "./components/TaskbarWidget";
import { StatusBar } from "./components/StatusBar";
import { UsageSummary } from "./components/UsageSummary";
import {
  createPresetRange,
  hasRolledOver,
  resolveDateRange,
  type DateRangeSelection,
} from "./dateRanges";
import type {
  DiagnosticsSnapshot,
  ProviderKey,
  ProviderSnapshot,
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
document.documentElement.classList.toggle("quick-panel-window", CURRENT_WINDOW_LABEL === "quick-panel");

/**
 * The two reads behind a provider fail independently — the quota windows can be current
 * while the history is not, and the reverse — so each is named separately rather than
 * collapsed into one message.
 */
function readErrors(provider: ProviderSnapshot): string[] {
  return [
    provider.liveError === null ? null : `Quota: ${provider.liveError}`,
    provider.historyError === null ? null : `History: ${provider.historyError}`,
  ].filter((message): message is string => message !== null);
}

function Dashboard() {
  const [usageRange, setUsageRange] = useState<UsageRangeSnapshot>(EMPTY_USAGE_RANGE);
  const [activeRange, setActiveRange] = useState<DateRangeSelection>(INITIAL_RANGE);
  const [rangeLoading, setRangeLoading] = useState(false);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSnapshot>(EMPTY_DIAGNOSTICS);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const activeRangeRef = useRef(INITIAL_RANGE);
  // The usage history is long, so it shows one provider at a time while the quota
  // sections above show them all.
  const [selectedProvider, setSelectedProvider] = useState<ProviderKey>("codex");
  const providerRef = useRef<ProviderKey>("codex");
  const rangeRequestId = useRef(0);
  const rangeRequested = useRef(false);

  const loadUsageRange = useCallback(async (range: DateRangeSelection, rangeProvider: ProviderKey) => {
    const resolvedRange = resolveDateRange(range);
    activeRangeRef.current = resolvedRange;
    setActiveRange(resolvedRange);
    const requestId = ++rangeRequestId.current;
    setRangeLoading(true);
    setRangeError(null);
    try {
      const next = await invoke<UsageRangeSnapshot>("get_usage_range", {
        provider: rangeProvider,
        startDate: resolvedRange.startDate,
        endDate: resolvedRange.endDate,
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
    // Each snapshot is also the only regular tick this window receives, so it is where a
    // calendar preset notices that midnight has passed. Without it an idle machine keeps
    // yesterday's totals under a heading that reads "Today" until something else asks for
    // a range.
    if (!rangeRequested.current || providerChanged || hasRolledOver(activeRangeRef.current)) {
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
    return () => {
      disposed = true;
      stopListening();
    };
  }, [loadUsageRange]);

  const showClaudeSettings = workspace.providers.some((provider) => provider.provider === "claude");
  const interfaceError = snapshotError ?? commandError;
  // The panel is behind a control now, so anything wrong inside it has to be visible from
  // outside it; otherwise a failed acquisition path is only found by looking for it.
  const diagnosticsAttention =
    interfaceError !== null ||
    diagnostics.watcher.status !== "active" ||
    diagnostics.acquisitions.some((acquisition) => acquisition.status === "failed");

  return (
    <main className="app-shell">
      <header className="app-header">
        {/* Each provider names itself on its own panel below, so the header does not list
            them a second time. */}
        <div className="identity">
          <h1>QuotaStation</h1>
        </div>
        <div className="header-actions">
          <button type="button" onClick={() => void refresh()} disabled={refreshing}>
            <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
            {refreshing ? "Refreshing" : "Refresh"}
          </button>
        </div>
      </header>
      {loaded && workspace.providers.length === 0 ? <ProviderSetup /> : null}
      <div className={`provider-grid${workspace.providers.length <= 1 ? " single" : ""}`}>
        {workspace.providers.map((provider) => (
          <section key={provider.provider} className="provider-panel">
            <header className="provider-panel-header">
              <h2>{provider.displayName}</h2>
              <span style={{ color: provider.compactStatus.color }}>{provider.compactStatus.label}</span>
            </header>
            {/* A reading held back as stale is only actionable with the reason beside it.
                The core redacts these before they leave it, so they are safe to draw. */}
            {readErrors(provider).map((message) => (
              <p className="provider-panel-error" key={message}>{message}</p>
            ))}
            <QuotaSection
              provider={provider.displayName}
              limits={provider.limits}
              earnedResetCount={provider.earnedResetCount}
              resets={provider.recentResets}
              statusColor={provider.compactStatus.color}
            />
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
        attention={diagnosticsAttention}
        onOpenSettings={() => setSettingsOpen(true)}
      />
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        showClaude={showClaudeSettings}
        diagnostics={diagnostics}
        providers={workspace.providers}
        interfaceError={interfaceError}
      />
    </main>
  );
}

export default function App() {
  if (CURRENT_WINDOW_LABEL === "quick-panel") return <QuickPanel initialWorkspace={EMPTY_WORKSPACE} />;
  if (CURRENT_WINDOW_LABEL === "taskbar-widget") return <TaskbarWidget initialWorkspace={EMPTY_WORKSPACE} />;
  return <Dashboard />;
}
