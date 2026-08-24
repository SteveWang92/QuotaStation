export type Freshness = "fresh" | "stale" | "unavailable";

/** Matches the Rust `ProviderKind`, which is also the database's provider key. */
export type ProviderKey = "codex" | "claude";

/**
 * Which history the dashboard is showing: one provider, or every provider counted
 * together. The combined view is a read of its own in the core, not a sum the renderer
 * assembles from separate answers.
 */
export type HistoryProvider = ProviderKey | "all";

export interface CompactStatus {
  level: "healthy" | "warning" | "critical" | "stale" | "unavailable";
  label: string;
}

/** Which palette the user asked for. `system` follows Windows and changes with it. */
export type ThemePreference = "system" | "dark" | "light";

/**
 * What each kind of window is drawn in right now. The taskbar widget follows the Windows
 * taskbar rather than the preference, so the two are answered separately.
 */
export interface ThemeSnapshot {
  app: "dark" | "light";
  taskbar: "dark" | "light";
}

export interface LimitWindow {
  kind: "primary" | "secondary";
  label: string;
  /** How loud this window's own reading is, on the thresholds every surface shares. */
  statusLevel: "healthy" | "warning" | "critical";
  usedPercent: number | null;
  windowDurationMins: number | null;
  resetsAt: number | null;
  source: "app_server" | "session_log" | "status_line";
  observedAt: number;
  freshness: Freshness;
}

export interface LimitResetEvent {
  windowKind: "primary" | "secondary";
  windowLabel: string;
  windowDurationMins: number;
  /** When the restarted window began, recovered from its new expiry. */
  anchoredAt: number;
  newResetsAt: number;
  previousResetsAt: number;
  usedPercentBefore: number;
  /**
   * Tokens recorded against the window this restart closed, or `null` when no hourly usage
   * ever covered it. Hourly buckets are the finest resolution behind it, so the total is
   * approximate at the two boundaries and the surfaces say so.
   */
  tokensInWindow: number | null;
  earlyBySeconds: number;
  classification: "scheduled" | "unplanned";
}

export interface ModelUsage {
  model: string;
  tokens: number;
  percent: number;
}

export interface TokenUsage {
  input: number;
  cacheRead: number;
  output: number;
  reasoning: number;
  total: number;
}

export interface ProviderSnapshot {
  provider: ProviderKey;
  displayName: string;
  /** The same name in the three characters a crowded row can spare. */
  shortName: string;
  /** Usage exists only because another device exported it; live quota is not local. */
  remoteUsageOnly: boolean;
  planType: string | null;
  limits: LimitWindow[];
  earnedResetCount: number | null;
  recentResets: LimitResetEvent[];
  today: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
  freshness: Freshness;
  /**
   * The snapshot mirrors the Rust type exactly, so several fields arrive already folded
   * into something a surface draws: the stale age is phrased inside `compactStatus`, the
   * attempt times are listed per acquisition path in the diagnostics panel, and the model
   * mix is drawn from the selected date range rather than from today alone.
   */
  staleAgeSeconds: number | null;
  compactStatus: CompactStatus;
  lastAttemptAt: string | null;
  lastLiveSuccessAt: string | null;
  lastHistorySuccessAt: string | null;
  /** Why each read last failed, named on the provider panel so a stale reading says why. */
  liveError: string | null;
  historyError: string | null;
  parserRevision: string;
  pricingCatalogRevision: string;
}

/**
 * Every provider in one payload. The surfaces show them together, so they are fetched
 * together and never drawn from two different moments.
 */
export interface WorkspaceSnapshot {
  providers: ProviderSnapshot[];
  aggregate: CompactStatus;
}

/**
 * Whether Claude Code hands its own quota windows to QuotaStation. Claude Code passes them
 * to the command configured as its status line and to nothing else, so this describes what
 * that setting currently holds.
 */
export interface ClaudeStatusLineStatus {
  installed: boolean;
  /** Whether a status line belonging to something else blocks installation. */
  hasForeignCommand: boolean;
  /** Epoch seconds of the last reading Claude Code handed over. */
  lastReadingAt: number | null;
  /** Claude Code is running, but only in hosts that render no status line. */
  desktopOnlySessions: boolean;
}

export interface DailyUsagePoint {
  date: string;
  usage: TokenUsage;
  apiEquivalentCostUsd: number | null;
  /** This day's own model mix, largest first, so a day can be opened without a new query. */
  models: ModelUsage[];
}

/** One local hour of usage, the hourly counterpart of `DailyUsagePoint`. */
export interface HourlyUsagePoint {
  /** The local hour this bucket opened, as `YYYY-MM-DDTHH:00`. */
  hourStart: string;
  usage: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
}

/** A range read hour by hour. Only the hours with usage are carried. */
export interface UsageHoursSnapshot {
  startDate: string;
  endDate: string;
  hours: HourlyUsagePoint[];
}

/** The highest share of a quota window observed on one local day. */
export interface QuotaHistoryPoint {
  date: string;
  peakUsedPercent: number;
}

export interface QuotaHistoryWindow {
  kind: "primary" | "secondary";
  label: string;
  points: QuotaHistoryPoint[];
}

export interface QuotaHistorySnapshot {
  startDate: string;
  endDate: string;
  windows: QuotaHistoryWindow[];
  /** Restarts anchored inside the range, oldest first. */
  resets: LimitResetEvent[];
}

export interface DeviceUsage {
  deviceId: string;
  displayName: string;
  local: boolean;
  tokens: number;
  percent: number;
}

export interface UsageRangeSnapshot {
  startDate: string;
  endDate: string;
  usage: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
  days: DailyUsagePoint[];
  devices: DeviceUsage[];
}

export interface AcquisitionDiagnostics {
  /** `<provider>_live` or `<provider>_history`. */
  acquisitionPath: string;
  label: string;
  status: "pending" | "succeeded" | "failed";
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  error: string | null;
}

export interface WatcherDiagnostics {
  status: "starting" | "active" | "degraded" | "unavailable";
  watchedLocationCount: number;
  lastEventAt: string | null;
  error: string | null;
}

export interface SharedFolderDiagnostics {
  status: "off" | "succeeded" | "failed";
  lastCompletedAt: string | null;
  error: string | null;
}

export interface DeviceDiagnostics {
  id: string;
  displayName: string;
  local: boolean;
  lastImportAt: string | null;
}

export interface DiagnosticsSnapshot {
  watcher: WatcherDiagnostics;
  acquisitions: AcquisitionDiagnostics[];
  retention: { status: string; lastCompletedAt: string | null; error: string | null };
  sharedFolder: SharedFolderDiagnostics;
  devices: DeviceDiagnostics[];
  parserRevision: string;
  pricingCatalogRevision: string;
  appVersion: string;
  buildCommit: string;
  /** debug, release portable, or release installed — which copy of QuotaStation this is. */
  buildKind: string;
}

/** How a provider is named where the name sits beside a reading rather than above one. */
export type ProviderLabelStyle = "short" | "full";

/** A display whose taskbar can host the status widget. */
export interface TaskbarDisplay {
  /** The Windows device name the choice is recorded as. */
  id: string;
  label: string;
  primary: boolean;
}

export interface AppSettings {
  theme: ThemePreference;
  taskbarWidgetEnabled: boolean;
  /** The chosen display's device name, or null for whichever taskbar is the primary one. */
  taskbarWidgetDisplay: string | null;
  statusLineProviderLabels: ProviderLabelStyle;
  statusLineOtherProviders: boolean;
  statusLineExtraDetails: boolean;
  notifyLowQuota: boolean;
  notifyReadFailures: boolean;
  notifyQuotaResets: boolean;
  /** Stable internal identity generated by the core; settings UI never edits it. */
  deviceId: string | null;
  /** This machine's name in device splits and diagnostics. */
  deviceName: string | null;
  /** Folder whose aggregate-only usage files are exchanged with other devices. */
  sharedUsageFolder: string | null;
}
