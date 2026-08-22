import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ProviderSetup } from "./components/ProviderSetup";
import { QuickPanel } from "./components/QuickPanel";
import { QuotaSection } from "./components/QuotaSection";
import { SettingsDialog } from "./components/SettingsDialog";
import { StatusBar } from "./components/StatusBar";
import { TaskbarWidget } from "./components/TaskbarWidget";
import { UsageSummary } from "./components/UsageSummary";
import {
  createPresetRange,
  type DateRangeSelection,
  hasRolledOver,
  isHourlyRange,
  previousPeriod,
  resolveDateRange,
} from "./dateRanges";
import { errorMessage } from "./errors";
import type {
  DiagnosticsSnapshot,
  HistoryProvider,
  ProviderSnapshot,
  QuotaHistorySnapshot,
  UsageHoursSnapshot,
  UsageRangeSnapshot,
  WorkspaceSnapshot,
} from "./types";
import { useSnapshot } from "./useSnapshot";
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
  appVersion: "",
  buildCommit: "",
  buildKind: "",
};

const CURRENT_WINDOW_LABEL = getCurrentWindow().label;
/**
 * The taskbar status is the one surface whose window can be built more than once — Explorer
 * destroys it when its taskbar is replaced — and each rebuild takes the next label, because
 * Tauri never gives the previous one back.
 */
const IS_TASKBAR_WIDGET = CURRENT_WINDOW_LABEL.startsWith("taskbar-widget");
document.documentElement.classList.toggle("compact-window", CURRENT_WINDOW_LABEL !== "main");
document.documentElement.classList.toggle("taskbar-window", IS_TASKBAR_WIDGET);
document.documentElement.classList.toggle(
  "quick-panel-window",
  CURRENT_WINDOW_LABEL === "quick-panel",
);

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
  // The comparison and the quota history are read for the same slice as the totals, so a
  // figure and the change beside it always describe the same two periods.
  const [previousRange, setPreviousRange] = useState<UsageRangeSnapshot | null>(null);
  // Hourly detail is read only for the ranges short enough to be drawn that way; `null`
  // is what puts the charts back on the daily axis.
  const [usageHours, setUsageHours] = useState<UsageHoursSnapshot | null>(null);
  const [quotaHistory, setQuotaHistory] = useState<QuotaHistorySnapshot | null>(null);
  const [activeRange, setActiveRange] = useState<DateRangeSelection>(INITIAL_RANGE);
  const [rangeLoading, setRangeLoading] = useState(false);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSnapshot>(EMPTY_DIAGNOSTICS);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const activeRangeRef = useRef(INITIAL_RANGE);
  // The usage history shows one provider at a time, or all of them counted together,
  // while the quota sections above always show each provider on its own. It opens on the
  // combined view: the window is read first for how much has been spent today, and naming
  // one provider there answers a narrower question than the one being asked. A workspace
  // with a single provider falls back to it, because "all" of one is that one.
  const [selectedProvider, setSelectedProvider] = useState<HistoryProvider>("all");
  const providerRef = useRef<HistoryProvider>("all");
  const rangeRequestId = useRef(0);
  const rangeRequested = useRef(false);

  const loadUsageRange = useCallback(
    async (range: DateRangeSelection, rangeProvider: HistoryProvider) => {
      const resolvedRange = resolveDateRange(range);
      activeRangeRef.current = resolvedRange;
      const requestId = ++rangeRequestId.current;
      setRangeLoading(true);
      setRangeError(null);
      const earlier = previousPeriod(resolvedRange);
      // The combined view names no provider, which the core reads as every provider at once.
      const provider = rangeProvider === "all" ? null : rangeProvider;
      try {
        const [next, previous, quota, hourly] = await Promise.all([
          invoke<UsageRangeSnapshot>("get_usage_range", {
            provider,
            startDate: resolvedRange.startDate,
            endDate: resolvedRange.endDate,
          }),
          invoke<UsageRangeSnapshot>("get_usage_range", {
            provider,
            startDate: earlier.startDate,
            endDate: earlier.endDate,
          }),
          // Quota is not summable: one provider's weekly window says nothing about
          // another's, so the combined view leaves that chart out rather than adding up
          // percentages of different allowances.
          provider === null
            ? Promise.resolve(null)
            : invoke<QuotaHistorySnapshot>("get_quota_history", {
                provider,
                startDate: resolvedRange.startDate,
                endDate: resolvedRange.endDate,
              }),
          // A longer range has more hours than the chart has pixels, and the core keeps
          // hourly rows only for the recent window anyway.
          isHourlyRange(resolvedRange.startDate, resolvedRange.endDate)
            ? invoke<UsageHoursSnapshot>("get_usage_hours", {
                provider,
                startDate: resolvedRange.startDate,
                endDate: resolvedRange.endDate,
              })
            : Promise.resolve(null),
        ]);
        if (requestId === rangeRequestId.current) {
          setUsageRange(next);
          setPreviousRange(previous);
          // Hourly rows only start existing at the first refresh after this build, and a
          // range that has usage but no hours would otherwise draw an empty hourly chart
          // over data the daily shape can show.
          setUsageHours(
            hourly === null || (hourly.hours.length === 0 && next.usage.total > 0) ? null : hourly,
          );
          setQuotaHistory(quota);
          setActiveRange(resolvedRange);
        }
      } catch (error) {
        if (requestId === rangeRequestId.current) setRangeError(errorMessage(error));
      } finally {
        if (requestId === rangeRequestId.current) setRangeLoading(false);
      }
    },
    [],
  );

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
  const onSnapshot = useCallback(
    (nextWorkspace: WorkspaceSnapshot) => {
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
    },
    [loadDiagnostics, loadUsageRange],
  );

  const { workspace, error: snapshotError, loaded } = useSnapshot(EMPTY_WORKSPACE, onSnapshot);
  const historyProvider =
    workspace.providers.find((provider) => provider.provider === selectedProvider) ??
    workspace.providers[0];

  const selectProvider = useCallback(
    (provider: HistoryProvider) => {
      providerRef.current = provider;
      setSelectedProvider(provider);
      void loadUsageRange(activeRangeRef.current, provider);
    },
    [loadUsageRange],
  );

  const selectRange = useCallback(
    (range: DateRangeSelection) => {
      activeRangeRef.current = range;
      setActiveRange(range);
      void loadUsageRange(range, providerRef.current);
    },
    [loadUsageRange],
  );

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      // refresh_now publishes the new snapshot through the shared subscription.
      await invoke("refresh_now");
      setCommandError(null);
      await Promise.all([
        loadUsageRange(activeRangeRef.current, providerRef.current),
        loadDiagnostics(),
      ]);
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
              <span style={{ color: provider.compactStatus.color }}>
                {provider.compactStatus.label}
              </span>
            </header>
            {/* A reading held back as stale is only actionable with the reason beside it.
                The core redacts these before they leave it, so they are safe to draw. */}
            {readErrors(provider).map((message) => (
              <p className="provider-panel-error" key={message}>
                {message}
              </p>
            ))}
            <QuotaSection
              provider={provider.displayName}
              limits={provider.limits}
              earnedResetCount={provider.earnedResetCount}
              resets={provider.recentResets}
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
          hours={usageHours}
          previousRange={previousRange}
          quotaHistory={quotaHistory}
          selection={activeRange}
          loading={rangeLoading}
          error={rangeError}
          onSelectRange={selectRange}
        />
      ) : null}
      <StatusBar attention={diagnosticsAttention} onOpenSettings={() => setSettingsOpen(true)} />
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
  if (CURRENT_WINDOW_LABEL === "quick-panel")
    return <QuickPanel initialWorkspace={EMPTY_WORKSPACE} />;
  if (IS_TASKBAR_WIDGET) return <TaskbarWidget initialWorkspace={EMPTY_WORKSPACE} />;
  return <Dashboard />;
}
