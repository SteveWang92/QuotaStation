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

/**
 * Whether a window is being spent faster or slower than it is elapsing. The core compares
 * the share used against the share of the window that has passed, so no surface decides it
 * for itself; `onTrack` is also what a window missing any part of that comparison reads.
 */
export type PaceLevel = "onTrack" | "ahead" | "behind";

export interface LimitWindow {
  kind: "primary" | "secondary";
  label: string;
  /** How loud this window's own reading is, on the thresholds every surface shares. */
  statusLevel: "healthy" | "warning" | "critical";
  pace: PaceLevel;
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
  /** How far apart the detections place the restart: the latest anchor minus the earliest. */
  anchorSpreadSeconds: number;
  /** Every device's detection of this restart, the one the fields above come from first. */
  detections: ResetDetection[];
}

/** One device's detection of a restart, with that device's own judgement of it. */
export interface ResetDetection {
  /** `null` for a restart recorded before detections were attributed to a device. */
  deviceName: string | null;
  local: boolean;
  source: "live" | "backfill";
  anchoredAt: number;
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
  /**
   * When the soonest of those earned resets stops being redeemable, or `null` where the
   * provider publishes only how many there are.
   */
  earnedResetExpiresAt: number | null;
  recentResets: LimitResetEvent[];
  today: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
  /**
   * The last seven local days of tokens, oldest first and today last, with a day nothing
   * was recorded on carried as a nought. The comparison with yesterday is its last pair.
   */
  dailyTotals: number[];
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
  /**
   * The provider is answering and says this machine is signed out. It is a state rather
   * than a failure, so the panel asks for a sign-in instead of reporting a broken source.
   */
  signInRequired: boolean;
  /**
   * The user switched this provider's quota off. No percentage is read or drawn; the usage
   * history below it is unaffected and carries on being collected and shown.
   */
  quotaDisabled: boolean;
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
  /** How far this computer's clock is behind internet time, in milliseconds; 0 unmeasured. */
  clockOffsetMs: number;
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

/**
 * A rolling window of hours: the totals and the hours they were summed from.
 *
 * A calendar range is answered from the stored days; a window of the last so many hours
 * cannot be, because the days at its two ends are partial. Both halves come from the same
 * hourly rows, so every figure on the surface describes the same hours.
 */
export interface UsageWindowSnapshot {
  range: UsageRangeSnapshot;
  hours: UsageHoursSnapshot;
}

/** One provider's whole recorded restart history, newest first. */
export interface ProviderResetHistory {
  provider: ProviderKey;
  displayName: string;
  resets: LimitResetEvent[];
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

/**
 * One session as the parser read it, with what the provider's own client said it cost
 * where the client said anything at all.
 *
 * Every session the parser has entries for is here. Claude Code records a cost of its own
 * only in recent sessions and Codex records none, so the client's side is optional.
 * Neither cost is a bill — nothing is charged per token on a subscription — so the pair,
 * where there is a pair, says whether the local estimate still tracks the vendor's own
 * accounting.
 */
export interface SessionCost {
  /** The client's own identifier. It never leaves this machine. */
  sessionId: string;
  sessionStartedAt: string;
  /** From the session's first entry to its last, measured the same way for every session. */
  durationMs: number;
  computedCostUsd: number;
  /** False once the client's own per-message costs priced the session, which makes both
      figures the same number and their agreement meaningless. */
  independent: boolean;
  /** All null for a session whose client recorded nothing. */
  reportedCostUsd: number | null;
  /** False when the client met a model it has no price for, so its total is short. */
  reportedComplete: boolean | null;
  apiDurationMs: number | null;
  linesAdded: number | null;
  linesRemoved: number | null;
  usage: TokenUsage;
  /** The models the session used, most expensive first. */
  models: string[];
}

export interface SessionCostSnapshot {
  /** Newest first. */
  sessions: SessionCost[];
  /** Both sums cover only the sessions the client also priced, so the two can be compared
      with each other. */
  reportedCostUsd: number;
  computedCostUsd: number;
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

export interface ClockDiagnostics {
  enabled: boolean;
  offsetMs: number;
  lastCheckedAt: string | null;
  error: string | null;
}

export interface DeviceDiagnostics {
  id: string;
  displayName: string;
  local: boolean;
  lastImportAt: string | null;
  /** How many quota restarts this device detected, as far as this machine knows. */
  restartCount: number;
}

export interface DiagnosticsSnapshot {
  watcher: WatcherDiagnostics;
  acquisitions: AcquisitionDiagnostics[];
  retention: { status: string; lastCompletedAt: string | null; error: string | null };
  sharedFolder: SharedFolderDiagnostics;
  clock: ClockDiagnostics;
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

/** How much room the quick panel takes for the same readings. */
export type QuickPanelDensity = "standard" | "compact";

/**
 * One segment of the Claude Code status line, named by the core — `model`, `branch`,
 * `quota:<provider>` and so on. The core sends every segment it can draw, so the list is
 * also the set the editor offers.
 */
export interface StatusLineSegment {
  id: string;
  enabled: boolean;
  /** 1 to 3. */
  row: number;
}

export interface StatusLineQuotaFormat {
  used: boolean;
  remaining: boolean;
  countdown: boolean;
  pace: boolean;
  bar: boolean;
}

export type StatusLineSeparators = "classic" | "arrow" | "powerline";
export type StatusLineColour = "full" | "quotaOnly" | "none";

export interface StatusLineLayout {
  /** In drawing order within each row. */
  segments: StatusLineSegment[];
  quota: StatusLineQuotaFormat;
  separators: StatusLineSeparators;
  colour: StatusLineColour;
}

/** A display whose taskbar can host the status widget. */
export interface TaskbarDisplay {
  /** The Windows device name the choice is recorded as. */
  id: string;
  label: string;
  primary: boolean;
}

/**
 * A provider whose quota this machine could read, for the switch that decides whether it
 * does. A provider with its quota switched off still has to be offered, so this comes from
 * the core rather than from the providers currently drawing a quota.
 */
export interface ProviderChoice {
  provider: ProviderKey;
  displayName: string;
}

export interface AppSettings {
  theme: ThemePreference;
  taskbarWidgetEnabled: boolean;
  /** The chosen display's device name, or null for whichever taskbar is the primary one. */
  taskbarWidgetDisplay: string | null;
  quickPanelDensity: QuickPanelDensity;
  statusLineProviderLabels: ProviderLabelStyle;
  statusLineLayout: StatusLineLayout;
  notifyLowQuota: boolean;
  notifyReadFailures: boolean;
  notifyQuotaResets: boolean;
  /** Stable internal identity generated by the core; settings UI never edits it. */
  deviceId: string | null;
  /** This machine's name in device splits and diagnostics. */
  deviceName: string | null;
  /**
   * The `provider:windowKind:newResetsAt` keys of the early-restart notes already read.
   * Keying on the expiry the note explains is what brings the note back at the next
   * restart and never brings back the one already dismissed.
   */
  dismissedResetNotices: string[];
  /**
   * The providers whose quota is not tracked, by provider key. Nothing reads their quota
   * and no surface draws it; their usage history is unaffected.
   */
  quotaDisabledProviders: string[];
  /** Folder whose aggregate-only usage files are exchanged with other devices. */
  sharedUsageFolder: string | null;
  /** The IANA zone chosen for every bucket and displayed time, or null to follow Windows. */
  timeZone: string | null;
  /** The zone in force, as the core resolved it. Read-only: not a choice. */
  resolvedTimeZone: string;
  /** The Windows zone, which `timeZone: null` follows. Read-only. */
  systemTimeZone: string;
  /** Whether this computer's clock is checked against internet time every two hours. */
  clockCheck: boolean;
}
