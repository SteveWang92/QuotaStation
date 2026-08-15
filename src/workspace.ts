import type { ProviderKey, ProviderSnapshot, WorkspaceSnapshot } from "./types";

/**
 * What a window shows before the core answers its first read. Every window mounts
 * before the core finishes opening its database, so each one needs a placeholder that
 * is shaped like a real snapshot rather than a null it has to guard on.
 */
export function emptySnapshot(
  provider: ProviderKey,
  displayName: string,
  shortName: string,
): ProviderSnapshot {
  return {
    provider,
    displayName,
    shortName,
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
      color: "#ff7469",
    },
    lastAttemptAt: null,
    lastLiveSuccessAt: null,
    lastHistorySuccessAt: null,
    liveError: null,
    historyError: null,
    parserRevision: "",
    pricingCatalogRevision: "",
  };
}

/** Keeps history queries attached to a provider that is still present in the workspace. */
export function resolveProviderKey(
  providers: ProviderSnapshot[],
  preferred: ProviderKey,
): ProviderKey | undefined {
  return providers.some((provider) => provider.provider === preferred)
    ? preferred
    : providers[0]?.provider;
}

/**
 * The placeholder carries no providers: which ones exist is decided by what the core
 * finds on this machine, so guessing one here would show a column that may not be there.
 */
export const EMPTY_WORKSPACE: WorkspaceSnapshot = {
  providers: [],
  aggregate: {
    level: "unavailable",
    label: "Starting",
    color: "#9aa4ad",
  },
};
