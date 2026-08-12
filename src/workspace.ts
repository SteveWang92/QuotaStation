import type { ProviderKey, ProviderSnapshot, WorkspaceSnapshot } from "./types";

/**
 * What a window shows before the core answers its first read. Every window mounts
 * before the core finishes opening its database, so each one needs a placeholder that
 * is shaped like a real snapshot rather than a null it has to guard on.
 */
export function emptySnapshot(provider: ProviderKey, displayName: string): ProviderSnapshot {
  return {
    provider,
    displayName,
    planType: null,
    limits: [],
    earnedResetCount: null,
    recentResets: [],
    today: { input: 0, cacheRead: 0, output: 0, reasoning: 0, total: 0 },
    apiEquivalentCostUsd: null,
    models: [],
    freshness: "unavailable",
    staleAgeSeconds: null,
    compactStatus: {
      level: "unavailable",
      label: "Provider unavailable",
      message: `No current ${displayName} quota data is available.`,
      color: "#ff7469",
    },
    lastAttemptAt: null,
    lastSuccessAt: null,
    liveError: null,
    historyError: null,
    parserRevision: "",
    pricingCatalogRevision: "",
  };
}

export const EMPTY_WORKSPACE: WorkspaceSnapshot = {
  providers: [emptySnapshot("codex", "Codex")],
  aggregate: {
    level: "unavailable",
    label: "Provider unavailable",
    message: "No current quota data is available.",
    color: "#ff7469",
  },
};
