import { invoke } from "@tauri-apps/api/core";
import { useCallback, useReducer, useRef } from "react";
import { logActivity } from "./activity";
import { hourlyUsageMatchesRange } from "./charts";
import {
  createAllRange,
  createPresetRange,
  type DateRangeSelection,
  hasRolledOver,
  isHourlyRange,
  previousPeriod,
  resolveDateRange,
  todayString,
} from "./dateRanges";
import { errorMessage } from "./errors";
import type {
  HistoryProvider,
  ProviderKey,
  ProviderSnapshot,
  QuotaHistorySnapshot,
  SessionCostSnapshot,
  UsageHoursSnapshot,
  UsageRangeSnapshot,
  UsageWindowSnapshot,
} from "./types";
import { resolveProviderKey } from "./workspace";

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

/**
 * One answer for the history section, however the range was expressed: the totals, the
 * period before them, and the hourly detail where there is any.
 */
interface RangeRead {
  range: UsageRangeSnapshot;
  previous: UsageRangeSnapshot | null;
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
  earlier: { startDate: string; endDate: string } | null,
): Promise<RangeRead> {
  const [next, previous, hourly] = await Promise.all([
    invoke<UsageRangeSnapshot>("get_usage_range", {
      provider,
      device,
      startDate: range.startDate,
      endDate: range.endDate,
    }),
    earlier === null
      ? Promise.resolve(null)
      : invoke<UsageRangeSnapshot>("get_usage_range", {
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

/**
 * What a read asks for. The three move together — picking a provider clears the device, and
 * every read carries all three — so they are one value rather than three.
 */
interface RangeRequest {
  selection: DateRangeSelection;
  /**
   * The usage history shows one provider at a time, or all of them counted together, while
   * the quota sections above always show each provider on its own. It opens on the combined
   * view: the window is read first for how much has been spent today, and naming one
   * provider there answers a narrower question than the one being asked. A workspace with a
   * single provider falls back to it, because "all" of one is that one.
   */
  provider: HistoryProvider;
  device: string | null;
}

/** Everything the history section draws, which is always one range read and nothing else. */
interface UsageRangeState extends RangeRequest {
  range: UsageRangeSnapshot;
  // The comparison and the quota history are read for the same slice as the totals, so a
  // figure and the change beside it always describe the same two periods.
  previous: UsageRangeSnapshot | null;
  // Hourly detail is read only for the ranges short enough to be drawn that way; `null`
  // is what puts the charts back on the daily axis.
  hours: UsageHoursSnapshot | null;
  quotaHistory: QuotaHistorySnapshot | null;
  // Session comparisons are placed by the day a session started, so they are read for the
  // same range as everything else and need no hourly counterpart.
  sessionCosts: SessionCostSnapshot | null;
  loading: boolean;
  error: string | null;
}

type UsageRangeAction =
  | { type: "asked"; request: RangeRequest; foreground: boolean }
  | {
      type: "loaded";
      selection: DateRangeSelection;
      usage: RangeRead;
      quotaHistory: QuotaHistorySnapshot | null;
      sessionCosts: SessionCostSnapshot | null;
    }
  | { type: "failed"; message: string };

const INITIAL_STATE: UsageRangeState = {
  selection: INITIAL_RANGE,
  provider: "all",
  device: null,
  range: EMPTY_USAGE_RANGE,
  previous: null,
  hours: null,
  quotaHistory: null,
  sessionCosts: null,
  loading: false,
  error: null,
};

function reduce(state: UsageRangeState, action: UsageRangeAction): UsageRangeState {
  switch (action.type) {
    // What was asked for is shown at once, so a pressed control answers before its data
    // arrives. A read the reader asked for also holds the charts at reduced opacity while it
    // runs, because they asked for something else and nothing else would say so. A
    // background read — the reconciliation poll, a session file that just changed — is
    // invisible until its data replaces what is on screen: Codex writes its logs while it
    // works, and dimming the dashboard every couple of seconds for a reload nobody asked for
    // is the flicker.
    case "asked":
      return { ...state, ...action.request, loading: action.foreground || state.loading };
    case "loaded":
      return {
        ...state,
        selection: action.selection,
        range: action.usage.range,
        previous: action.usage.previous,
        hours: action.usage.hours,
        quotaHistory: action.quotaHistory,
        sessionCosts: action.sessionCosts,
        loading: false,
        error: null,
      };
    case "failed":
      return { ...state, loading: false, error: action.message };
  }
}

/**
 * The usage history: one range read, the controls that change what it asks for, and the
 * three things outside the section that also start a read — the refresh button, the core's
 * `history-updated` event, and each workspace snapshot.
 */
export function useUsageRange() {
  const [state, dispatch] = useReducer(reduce, INITIAL_STATE);
  // What the next read will ask for. Held beside the state because a click, a core event and
  // a snapshot all start a read from a callback that has not been re-rendered yet, and each
  // of them changes one part of the request while keeping the other two.
  const request = useRef<RangeRequest>(INITIAL_STATE);
  // Whichever read arrives last owns the display, so an earlier one that is still in flight
  // neither draws its answer nor releases the hold a later one is keeping.
  const readId = useRef(0);
  // A workspace with no providers means something different before and after the first read
  // was asked for, and only the first one is allowed to show the reader it is loading.
  const asked = useRef(false);

  const load = useCallback(async (next: RangeRequest, options?: { background?: boolean }) => {
    let selection = resolveDateRange(next.selection);
    request.current = { ...next, selection };
    asked.current = true;
    const id = ++readId.current;
    const foreground = options?.background !== true;
    dispatch({ type: "asked", request: request.current, foreground });
    // The combined view names no provider, which the core reads as every provider at once.
    const provider = next.provider === "all" ? null : next.provider;
    const device = next.device;
    try {
      if (next.selection.preset === "all") {
        const firstUsageDate = await invoke<string | null>("get_usage_start_date", {
          provider,
          device,
        });
        if (id !== readId.current) return;
        selection = createAllRange(firstUsageDate ?? todayString());
        request.current = { ...request.current, selection };
      }
      const earlier = selection.preset === "all" ? null : previousPeriod(selection);
      const window = hourBounds(selection);
      const earlierWindow = earlier === null ? null : hourBounds(earlier);
      const [usage, quotaHistory, sessionCosts] = await Promise.all([
        window !== null && earlierWindow !== null
          ? readRollingWindow(provider, device, window, earlierWindow)
          : readCalendarRange(provider, device, selection, earlier),
        // Quota is not summable: one provider's weekly window says nothing about another's,
        // so the combined view leaves that chart out rather than adding up percentages of
        // different allowances. It is measured once a poll and summarised by the day, so a
        // rolling window reads the days it touches like any other range.
        provider === null
          ? Promise.resolve(null)
          : invoke<QuotaHistorySnapshot>("get_quota_history", {
              provider,
              startDate: selection.startDate,
              endDate: selection.endDate,
            }),
        invoke<SessionCostSnapshot>("get_session_costs", {
          provider,
          startDate: selection.startDate,
          endDate: selection.endDate,
        }),
      ]);
      if (id === readId.current) {
        dispatch({ type: "loaded", selection, usage, quotaHistory, sessionCosts });
      }
    } catch (error) {
      if (id === readId.current) dispatch({ type: "failed", message: errorMessage(error) });
    }
  }, []);

  const selectProvider = useCallback(
    (provider: HistoryProvider) => {
      logActivity(`history provider set to ${provider}`);
      void load({ selection: request.current.selection, provider, device: null });
    },
    [load],
  );

  const selectDevice = useCallback(
    (device: string | null) => {
      logActivity(`history device set to ${device === null ? "every device" : "one device"}`);
      void load({ ...request.current, device });
    },
    [load],
  );

  const selectRange = useCallback(
    (selection: DateRangeSelection) => {
      logActivity(`history range set to ${selection.preset}`);
      void load({ ...request.current, selection });
    },
    [load],
  );

  /** Reads the same range again, for the refresh button and the core's history event. */
  const reload = useCallback(
    (options?: { background?: boolean }) => load(request.current, options),
    [load],
  );

  /**
   * Answers a workspace snapshot.
   *
   * Each snapshot is also the only regular tick this window receives, so it is where a
   * calendar preset notices that midnight has passed. Without it an idle machine keeps
   * yesterday's totals under a heading that reads "Today" until something else asks for a
   * range. Only a repeat read is silent: the first one has nothing on screen to keep still,
   * and hiding it would show an empty range until it arrived.
   */
  const followWorkspace = useCallback(
    (providers: ProviderSnapshot[]) => {
      const provider = resolveProviderKey(providers, request.current.provider);
      if (!provider) return;
      const providerChanged = provider !== request.current.provider;
      const firstRead = !asked.current;
      if (!firstRead && !providerChanged && !hasRolledOver(request.current.selection)) return;
      void load({ ...request.current, provider }, { background: !firstRead });
    },
    [load],
  );

  return { usage: state, selectProvider, selectDevice, selectRange, reload, followWorkspace };
}
