export type Freshness = "fresh" | "stale" | "unavailable";

export interface LimitWindow {
  kind: "primary" | "secondary";
  label: string;
  usedPercent: number | null;
  remainingPercent: number | null;
  windowDurationMins: number | null;
  resetsAt: number | null;
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
  provider: "codex";
  planType: string | null;
  limits: LimitWindow[];
  earnedResetCount: number | null;
  recentResets: LimitResetEvent[];
  today: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
  freshness: Freshness;
  staleAgeSeconds: number | null;
  compactStatus: {
    level: "healthy" | "warning" | "critical" | "stale" | "unavailable";
    label: string;
    message: string;
    color: string;
  };
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  liveError: string | null;
  historyError: string | null;
  parserRevision: string;
  pricingCatalogRevision: string;
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
  acquisitionPath: "codex_live" | "codex_history";
  label: string;
  status: "pending" | "succeeded" | "failed";
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  error: string | null;
}

export interface WatcherDiagnostics {
  status: "starting" | "active" | "unavailable";
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
}
