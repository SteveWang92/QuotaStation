import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowLeft, RefreshCw, SlidersHorizontal } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { logActivity } from "./activity";
import { watchAppSettings } from "./appSettings";
import { ProviderSetup } from "./components/ProviderSetup";
import { QuickPanel } from "./components/QuickPanel";
import { QuotaSection, resetNoticeKey } from "./components/QuotaSection";
import { SettingsPage } from "./components/SettingsPage";
import { TaskbarWidget } from "./components/TaskbarWidget";
import { UsageSummary } from "./components/UsageSummary";
import { errorMessage } from "./errors";
import { formatClockOffset } from "./format";
import { statusColor, watchTheme } from "./theme";
import type { DiagnosticsSnapshot, ProviderSnapshot, WorkspaceSnapshot } from "./types";
import { useSnapshot } from "./useSnapshot";
import { useUsageRange } from "./useUsageRange";
import { onScreen } from "./visible";
import { EMPTY_WORKSPACE } from "./workspace";

const EMPTY_DIAGNOSTICS: DiagnosticsSnapshot = {
  watcher: { status: "starting", watchedLocationCount: 0, lastEventAt: null, error: null },
  acquisitions: [],
  retention: { status: "pending", lastCompletedAt: null, error: null },
  sharedFolder: { status: "off", lastCompletedAt: null, error: null },
  clock: { enabled: false, offsetMs: 0, lastCheckedAt: null, error: null },
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
watchAppSettings();

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

function Dashboard() {
  const { usage, selectProvider, selectDevice, selectRange, reload, followWorkspace } =
    useUsageRange();
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [eventError, setEventError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSnapshot>(EMPTY_DIAGNOSTICS);
  // Settings is a page rather than an overlay: it is read and worked through — a source
  // set up, then checked, then the restart history read — and a dialog over the dashboard
  // both hides what it is being compared against and has nowhere to put a long list.
  const [showSettings, setShowSettings] = useState(false);

  const loadDiagnostics = useCallback(async () => {
    try {
      setDiagnostics(await invoke<DiagnosticsSnapshot>("get_diagnostics"));
      setDiagnosticsError(null);
    } catch (error) {
      setDiagnosticsError(errorMessage(error));
    }
  }, []);

  // The shared subscription retries until the core is ready, so the first usage
  // range read waits for it instead of failing against an unmanaged state.
  const readForSnapshot = useCallback(
    (nextWorkspace: WorkspaceSnapshot) => {
      void loadDiagnostics();
      followWorkspace(nextWorkspace.providers);
    },
    [loadDiagnostics, followWorkspace],
  );

  // A dismissed dashboard is hidden rather than closed, and the core goes on sending it every
  // snapshot. Answering one costs a diagnostics read and a whole range read — several
  // database queries — for a window nobody is looking at, so the reads wait until it is back
  // on screen and are then made against the newest snapshot rather than the one that was
  // skipped.
  // The core's history event is answered the same way: the range read it asks for waits for
  // the window to be shown again.
  const latestWorkspace = useRef(EMPTY_WORKSPACE);
  const readsDeferred = useRef(false);
  const historyDeferred = useRef(false);

  const onSnapshot = useCallback(
    (nextWorkspace: WorkspaceSnapshot) => {
      latestWorkspace.current = nextWorkspace;
      void onScreen().then((visible) => {
        if (visible) readForSnapshot(nextWorkspace);
        else readsDeferred.current = true;
      });
    },
    [readForSnapshot],
  );

  const { workspace, error: snapshotError, loaded } = useSnapshot(EMPTY_WORKSPACE, onSnapshot);
  const historyProvider =
    workspace.providers.find((provider) => provider.provider === usage.provider) ??
    workspace.providers[0];

  const refresh = useCallback(async () => {
    logActivity("refresh pressed on the dashboard");
    setRefreshing(true);
    try {
      // refresh_now publishes the new snapshot through the shared subscription.
      await invoke("refresh_now");
      setRefreshError(null);
      await Promise.all([reload(), loadDiagnostics()]);
    } catch (error) {
      setRefreshError(errorMessage(error));
    } finally {
      setRefreshing(false);
    }
  }, [loadDiagnostics, reload]);

  useEffect(() => {
    let disposed = false;
    const stops: Array<() => void> = [];
    const keep = (unlisten: () => void) => {
      if (disposed) unlisten();
      else stops.push(unlisten);
    };
    const failed = (error: unknown) => {
      if (!disposed) setEventError(errorMessage(error));
    };
    void listen("history-updated", () => {
      void onScreen().then((visible) => {
        if (visible) void reload({ background: true });
        else historyDeferred.current = true;
      });
    })
      .then(keep)
      .catch(failed);
    // Showing the dashboard focuses it, so this is where a window that was hidden catches up
    // on the snapshots and history updates it let pass.
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) return;
        if (readsDeferred.current) {
          readsDeferred.current = false;
          readForSnapshot(latestWorkspace.current);
        }
        if (historyDeferred.current) {
          historyDeferred.current = false;
          void reload({ background: true });
        }
      })
      .then(keep)
      .catch(failed);
    return () => {
      disposed = true;
      for (const stop of stops) stop();
    };
  }, [reload, readForSnapshot]);

  const showClaudeSettings = workspace.providers.some(
    (provider) => provider.provider === "claude" && !provider.remoteUsageOnly,
  );
  const interfaceError = snapshotError ?? refreshError ?? eventError ?? diagnosticsError;
  // The panel is behind a control now, so anything wrong inside it has to be visible from
  // outside it; otherwise a failed acquisition path is only found by looking for it.
  // Every quota window on display right now, in the vocabulary the dismissed early-restart
  // notes are recorded in. Rewriting the record against these is what stops it growing:
  // a note for a window nobody is looking at any more can never be shown again either way.
  const quotaProviders = workspace.providers.filter((provider) => !provider.quotaDisabled);
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
            onClick={() => {
              logActivity(showSettings ? "settings closed" : "settings opened");
              setShowSettings((open) => !open);
            }}
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
          {/* A clock this far off makes every countdown and every reading's time wrong by
              as much, which is worth saying above everything it affects. */}
          {formatClockOffset(workspace.clockOffsetMs) ? (
            <p className="clock-banner" role="status">
              {formatClockOffset(workspace.clockOffsetMs)}. Countdowns are corrected; fix the
              Windows clock to correct the times Codex and Claude Code record.
            </p>
          ) : null}
          {loaded && workspace.providers.length === 0 ? <ProviderSetup /> : null}
          {/* The grid is the quota display, so a provider whose quota is switched off has
              no panel here at all. Its usage keeps its place in the history below. */}
          {loaded && workspace.providers.length > 0 && quotaProviders.length === 0 ? (
            <p className="provider-quota-note">
              Quota tracking is off for every provider. Switch one back on in Settings to see its
              quota here; the usage below is unaffected.
            </p>
          ) : null}
          <div className={`provider-grid${quotaProviders.length <= 1 ? " single" : ""}`}>
            {quotaProviders.map((provider) => (
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
                ) : provider.signInRequired ? (
                  // Nothing here is broken and nothing is worth retrying quickly, so the
                  // panel says what to do about it rather than showing windows it cannot
                  // read or promising a retry that cannot succeed.
                  <p className="provider-quota-note">
                    {provider.displayName} is signed out, or its sign-in has expired. Sign in with
                    its own client again and the quota comes back on its own — QuotaStation checks
                    once an hour until then. You can also switch its quota off in Settings.
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
              activeProvider={usage.provider}
              onSelectProvider={selectProvider}
              activeDevice={usage.device}
              onSelectDevice={selectDevice}
              knownDevices={diagnostics.devices}
              range={usage.range}
              hours={usage.hours}
              previousRange={usage.previous}
              quotaHistory={usage.quotaHistory}
              sessionCosts={usage.sessionCosts}
              selection={usage.selection}
              loading={usage.loading}
              error={usage.error}
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
