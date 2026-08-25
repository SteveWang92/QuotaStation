import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowLeft, RefreshCw, SlidersHorizontal } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { hourlyUsageMatchesRange } from "./charts";
import { ProviderSetup } from "./components/ProviderSetup";
import { QuickPanel } from "./components/QuickPanel";
import { QuotaSection, resetNoticeKey } from "./components/QuotaSection";
import { SettingsPage } from "./components/SettingsPage";
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
import { statusColor, watchTheme } from "./theme";
import type {
  DiagnosticsSnapshot,
  HistoryProvider,
  ProviderKey,
  ProviderSnapshot,
  QuotaHistorySnapshot,
  UsageHoursSnapshot,
  UsageRangeSnapshot,
  UsageWindowSnapshot,
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
  devices: [],
};

const EMPTY_DIAGNOSTICS: DiagnosticsSnapshot = {
  watcher: { status: "starting", watchedLocationCount: 0, lastEventAt: null, error: null },
  acquisitions: [],
  retention: { status: "pending", lastCompletedAt: null, error: null },
  sharedFolder: { status: "off", lastCompletedAt: null, error: null },
  devices: [],
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
watchTheme(IS_TASKBAR_WIDGET);

/**
 * The two reads behind a provider fail independently — the quota windows can be current
 * while the history is not, and the reverse — so each is named separately rather than
 * collapsed into one message.
 */
function readErrors(provider: ProviderSnapshot): string[] {
  return [
    provider.remoteUsageOnly || provider.liveError === null ? null : `Quota: ${provider.liveError}`,
    provider.historyError === null ? null : `History: ${provider.historyError}`,
  ].filter((message): message is string => message !== null);
}

/**
 * One answer for the history section, however the range was expressed: the totals, the
 * period before them, and the hourly detail where there is any.
 */
interface RangeRead {
  range: UsageRangeSnapshot;
  previous: UsageRangeSnapshot;
  hours: UsageHoursSnapshot | null;
}

/** The hour bounds of a rolling window, or `null` for a range expressed in whole days. */
function hourBounds(period: {
  startHour?: string;
  endHour?: string;
}): { startHour: string; endHour: string } | null {
  return period.startHour === undefined || period.endHour === undefined
    ? null
    : { startHour: period.startHour, endHour: period.endHour };
}

async function readCalendarRange(
  provider: ProviderKey | null,
  device: string | null,
  range: DateRangeSelection,
  earlier: { startDate: string; endDate: string },
): Promise<RangeRead> {
  const [next, previous, hourly] = await Promise.all([
    invoke<UsageRangeSnapshot>("get_usage_range", {
      provider,
      device,
      startDate: range.startDate,
      endDate: range.endDate,
    }),
    invoke<UsageRangeSnapshot>("get_usage_range", {
      provider,
      device,
      startDate: earlier.startDate,
      endDate: earlier.endDate,
    }),
    // A longer range has more hours than the chart has pixels, and the core keeps
    // hourly rows only for the recent window anyway.
    isHourlyRange(range.startDate, range.endDate)
      ? invoke<UsageHoursSnapshot>("get_usage_hours", {
          provider,
          device,
          startDate: range.startDate,
          endDate: range.endDate,
        })
      : Promise.resolve(null),
  ]);
  // Hourly rows only start existing after each provider's first refresh on this build.
  // Until every provider covers the whole range, keep the complete daily shape instead of
  // drawing a plausible but partial hourly chart.
  const hours = hourly !== null && hourlyUsageMatchesRange(hourly, next) ? hourly : null;
  return { range: next, previous, hours };
}

/**
 * The rolling window is one read rather than three: its totals are summed from the same
 * hourly rows the chart draws, because the two calendar days it touches are both partial
 * and neither the day rows nor the device split could be taken from them.
 */
async function readRollingWindow(
  provider: ProviderKey | null,
  device: string | null,
  window: { startHour: string; endHour: string },
  earlier: { startHour: string; endHour: string },
): Promise<RangeRead> {
  const [next, previous] = await Promise.all([
    invoke<UsageWindowSnapshot>("get_usage_window", { provider, device, ...window }),
    invoke<UsageWindowSnapshot>("get_usage_window", { provider, device, ...earlier }),
  ]);
  return { range: next.range, previous: previous.range, hours: next.hours };
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
  // Settings is a page rather than an overlay: it is read and worked through — a source
  // set up, then checked, then the restart history read — and a dialog over the dashboard
  // both hides what it is being compared against and has nowhere to put a long list.
  const [showSettings, setShowSettings] = useState(false);
  const activeRangeRef = useRef(INITIAL_RANGE);
  // The usage history shows one provider at a time, or all of them counted together,
  // while the quota sections above always show each provider on its own. It opens on the
  // combined view: the window is read first for how much has been spent today, and naming
  // one provider there answers a narrower question than the one being asked. A workspace
  // with a single provider falls back to it, because "all" of one is that one.
  const [selectedProvider, setSelectedProvider] = useState<HistoryProvider>("all");
  const providerRef = useRef<HistoryProvider>("all");
  const rangeRequestId = useRef(0);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const deviceRef = useRef<string | null>(null);
  const rangeRequested = useRef(false);

  const loadUsageRange = useCallback(
    async (range: DateRangeSelection, rangeProvider: HistoryProvider, device: string | null) => {
      const resolvedRange = resolveDateRange(range);
      activeRangeRef.current = resolvedRange;
      const requestId = ++rangeRequestId.current;
      setRangeLoading(true);
      setRangeError(null);
      const earlier = previousPeriod(resolvedRange);
      // The combined view names no provider, which the core reads as every provider at once.
      const provider = rangeProvider === "all" ? null : rangeProvider;
      const window = hourBounds(resolvedRange);
      const earlierWindow = hourBounds(earlier);
      try {
        const [usage, quota] = await Promise.all([
          window !== null && earlierWindow !== null
            ? readRollingWindow(provider, device, window, earlierWindow)
            : readCalendarRange(provider, device, resolvedRange, earlier),
          // Quota is not summable: one provider's weekly window says nothing about
          // another's, so the combined view leaves that chart out rather than adding up
          // percentages of different allowances. It is measured once a poll and summarised
          // by the day, so a rolling window reads the days it touches like any other range.
          provider === null
            ? Promise.resolve(null)
            : invoke<QuotaHistorySnapshot>("get_quota_history", {
                provider,
                startDate: resolvedRange.startDate,
                endDate: resolvedRange.endDate,
              }),
        ]);
        if (requestId === rangeRequestId.current) {
          setUsageRange(usage.range);
          setPreviousRange(usage.previous);
          setUsageHours(usage.hours);
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
        void loadUsageRange(activeRangeRef.current, rangeProvider, deviceRef.current);
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
      deviceRef.current = null;
      setSelectedProvider(provider);
      setSelectedDevice(null);
      void loadUsageRange(activeRangeRef.current, provider, null);
    },
    [loadUsageRange],
  );

  const selectDevice = useCallback(
    (device: string | null) => {
      deviceRef.current = device;
      setSelectedDevice(device);
      void loadUsageRange(activeRangeRef.current, providerRef.current, device);
    },
    [loadUsageRange],
  );

  const selectRange = useCallback(
    (range: DateRangeSelection) => {
      activeRangeRef.current = range;
      setActiveRange(range);
      void loadUsageRange(range, providerRef.current, deviceRef.current);
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
        loadUsageRange(activeRangeRef.current, providerRef.current, deviceRef.current),
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
      void loadUsageRange(activeRangeRef.current, providerRef.current, deviceRef.current);
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

  const showClaudeSettings = workspace.providers.some(
    (provider) => provider.provider === "claude" && !provider.remoteUsageOnly,
  );
  const interfaceError = snapshotError ?? commandError;
  // The panel is behind a control now, so anything wrong inside it has to be visible from
  // outside it; otherwise a failed acquisition path is only found by looking for it.
  // Every quota window on display right now, in the vocabulary the dismissed early-restart
  // notes are recorded in. Rewriting the record against these is what stops it growing:
  // a note for a window nobody is looking at any more can never be shown again either way.
  const liveWindowKeys = workspace.providers.flatMap((provider) =>
    provider.limits.map((limit) => resetNoticeKey(provider.provider, limit.kind, limit.resetsAt)),
  );
  const diagnosticsAttention =
    interfaceError !== null ||
    diagnostics.watcher.status !== "active" ||
    diagnostics.acquisitions.some((acquisition) => acquisition.status === "failed") ||
    diagnostics.sharedFolder.status === "failed";

  return (
    <main className={`app-shell${showSettings ? " settings-open" : ""}`}>
      <header className="app-header">
        {/* Each provider names itself on its own panel below, so the header does not list
            them a second time. */}
        <div className="identity">
          <h1>{showSettings ? "Settings" : "QuotaStation"}</h1>
        </div>
        <div className="header-actions">
          <button type="button" onClick={() => void refresh()} disabled={refreshing}>
            <RefreshCw aria-hidden="true" className={refreshing ? "spinning" : ""} />
            {refreshing ? "Refreshing" : "Refresh"}
          </button>
          {/* The settings page has no control of its own to come back from, so the one
              that opened it is the one that closes it. Anything wrong inside it has to be
              visible from out here, or a failed acquisition path is only found by looking
              for it. */}
          <button
            type="button"
            className={diagnosticsAttention && !showSettings ? "attention" : ""}
            onClick={() => setShowSettings((open) => !open)}
          >
            {showSettings ? (
              <>
                <ArrowLeft aria-hidden="true" /> Dashboard
              </>
            ) : (
              <>
                <SlidersHorizontal aria-hidden="true" /> Settings
              </>
            )}
          </button>
        </div>
      </header>
      {showSettings ? (
        <SettingsPage
          showClaude={showClaudeSettings}
          diagnostics={diagnostics}
          providers={workspace.providers}
          interfaceError={interfaceError}
        />
      ) : (
        <>
          {loaded && workspace.providers.length === 0 ? <ProviderSetup /> : null}
          <div className={`provider-grid${workspace.providers.length <= 1 ? " single" : ""}`}>
            {workspace.providers.map((provider) => (
              <section key={provider.provider} className="provider-panel">
                <header className="provider-panel-header">
                  <h2>{provider.displayName}</h2>
                  <span style={{ color: statusColor(provider.compactStatus) }}>
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
                {provider.remoteUsageOnly ? (
                  <p className="provider-quota-note">
                    Usage is synced from another device. Quota can only be read on that device.
                  </p>
                ) : (
                  <QuotaSection
                    provider={provider.provider}
                    providerName={provider.displayName}
                    limits={provider.limits}
                    earnedResetCount={provider.earnedResetCount}
                    earnedResetExpiresAt={provider.earnedResetExpiresAt}
                    resets={provider.recentResets}
                    liveWindowKeys={liveWindowKeys}
                  />
                )}
              </section>
            ))}
          </div>
          {historyProvider ? (
            <UsageSummary
              snapshot={historyProvider}
              providers={workspace.providers}
              activeProvider={selectedProvider}
              onSelectProvider={selectProvider}
              activeDevice={selectedDevice}
              onSelectDevice={selectDevice}
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
        </>
      )}
    </main>
  );
}

export default function App() {
  if (CURRENT_WINDOW_LABEL === "quick-panel")
    return <QuickPanel initialWorkspace={EMPTY_WORKSPACE} />;
  if (IS_TASKBAR_WIDGET) return <TaskbarWidget initialWorkspace={EMPTY_WORKSPACE} />;
  return <Dashboard />;
}
