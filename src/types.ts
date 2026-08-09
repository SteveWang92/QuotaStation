export type Freshness = "fresh" | "stale" | "unavailable";

export interface LimitWindow {
  kind: "primary" | "secondary";
  label: string;
  usedPercent: number | null;
  windowDurationMins: number | null;
  resetsAt: number | null;
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
  today: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
  freshness: Freshness;
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
