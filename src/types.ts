export type Freshness = "fresh" | "stale" | "unavailable";

/** Matches the Rust `ProviderKind`, which is also the database's provider key. */
export type ProviderKey = "codex" | "claude";

export interface CompactStatus {
  level: "healthy" | "warning" | "critical" | "stale" | "unavailable";
  label: string;
  color: string;
}

export interface LimitWindow {
  kind: "primary" | "secondary";
  label: string;
  /** This window's own colour, from the same thresholds the provider status uses. */
  statusColor: string;
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
}

export interface UsageRangeSnapshot {
  startDate: string;
  endDate: string;
  usage: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
  days: DailyUsagePoint[];
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

export interface DiagnosticsSnapshot {
  watcher: WatcherDiagnostics;
  acquisitions: AcquisitionDiagnostics[];
  retention: { status: string; lastCompletedAt: string | null; error: string | null };
  parserRevision: string;
  pricingCatalogRevision: string;
  appVersion: string;
  /** debug, release portable, or release installed — which copy of QuotaStation this is. */
  buildKind: string;
}

/** How a provider is named where the name sits beside a reading rather than above one. */
export type ProviderLabelStyle = "short" | "full";

export interface AppSettings {
  taskbarWidgetEnabled: boolean;
  statusLineProviderLabels: ProviderLabelStyle;
  statusLineOtherProviders: boolean;
  statusLineExtraDetails: boolean;
}
