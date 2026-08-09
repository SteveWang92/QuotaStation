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
