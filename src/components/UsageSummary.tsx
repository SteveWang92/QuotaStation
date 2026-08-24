import { CalendarDays, X } from "lucide-react";
import { useMemo, useState } from "react";
import { alignToBuckets, calendarDays, calendarHours } from "../charts";
import {
  createCustomRange,
  createPresetRange,
  customRangeTooLong,
  type DateRangeSelection,
  formatRangeDate,
  MAX_CUSTOM_RANGE_DAYS,
  type RangePreset,
  todayString,
  toLocalDateString,
} from "../dateRanges";
import {
  formatCompactCurrency,
  formatCompactNumber,
  formatCurrency,
  formatDelta,
  formatNumber,
  formatRevision,
} from "../format";
import { SERIES_LIMIT, SERIES_REST, SERIES_SLOTS } from "../series";
import type {
  DailyUsagePoint,
  HistoryProvider,
  ModelUsage,
  ProviderSnapshot,
  QuotaHistorySnapshot,
  TokenUsage,
  UsageHoursSnapshot,
  UsageRangeSnapshot,
} from "../types";
import { type ChartMarker, type ChartSeries, TrendChart } from "./TrendChart";

/**
 * What a chart reads off one bucket. A day and an hour carry the same three figures, and
 * the charts never need to know which of the two they were handed.
 */
interface UsagePoint {
  usage: TokenUsage;
  apiEquivalentCostUsd: number | null;
  models: ModelUsage[];
}

const PRESETS: Array<{ value: Exclude<RangePreset, "custom">; label: string }> = [
  { value: "today", label: "Today" },
  { value: "3d", label: "3d" },
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
];

/**
 * The four token categories, in the order they are stacked and listed. The order is also
 * the colour order, so a category keeps one colour on the chart, in the tooltip and in the
 * breakdown beside it.
 */
const CATEGORIES: Array<{ key: keyof TokenUsage; label: string; color: string }> = [
  { key: "input", label: "Input", color: SERIES_SLOTS[0] },
  { key: "output", label: "Output", color: SERIES_SLOTS[1] },
  { key: "cacheRead", label: "Cached input", color: SERIES_SLOTS[2] },
  { key: "reasoning", label: "Reasoning", color: SERIES_SLOTS[3] },
];

interface UsageSummaryProps {
  snapshot: ProviderSnapshot;
  /** All enabled providers, so the history can be switched between them. */
  providers: ProviderSnapshot[];
  activeProvider: HistoryProvider;
  onSelectProvider: (provider: HistoryProvider) => void;
  activeDevice: string | null;
  onSelectDevice: (device: string | null) => void;
  range: UsageRangeSnapshot;
  /** The same range hour by hour, when it is short enough to be read that way. */
  hours: UsageHoursSnapshot | null;
  /** The period of the same length immediately before this one, for the comparison. */
  previousRange: UsageRangeSnapshot | null;
  quotaHistory: QuotaHistorySnapshot | null;
  selection: DateRangeSelection;
  loading: boolean;
  error: string | null;
  onSelectRange: (range: DateRangeSelection) => void;
}

/**
 * Folds a model list down to the slots the palette has, with everything past them summed
 * into one "Other" entry. A generated fifth colour would be indistinguishable from one of
 * the four above it, so the tail is named instead of coloured.
 */
function namedModels(models: ModelUsage[]): string[] {
  return models.slice(0, SERIES_LIMIT).map((model) => model.model);
}

export function UsageSummary({
  snapshot,
  providers,
  activeProvider,
  onSelectProvider,
  activeDevice,
  onSelectDevice,
  range,
  hours,
  previousRange,
  quotaHistory,
  selection,
  loading,
  error,
  onSelectRange,
}: UsageSummaryProps) {
  const [showCustom, setShowCustom] = useState(false);
  const [customStart, setCustomStart] = useState(selection.startDate);
  const [customEnd, setCustomEnd] = useState(selection.endDate);
  // Opening a day narrows the breakdown cards to it; the charts and the totals above stay
  // on the whole range, so the day is always read against its own context.
  const [openDay, setOpenDay] = useState<string | null>(null);
  // The combined view belongs to no provider, so anything that describes one — its plan,
  // its pricing catalogue, its quota windows — is left off rather than borrowed from
  // whichever provider happens to be first.
  const combined = activeProvider === "all";

  // A short range is read hour by hour: three columns describe three days without saying
  // anything about when the work in them happened. The core decides what it can answer
  // hourly, so an hourly reading arriving is what puts the charts in that resolution.
  const hourly = hours !== null;
  const days = useMemo(
    () => calendarDays(range.startDate, range.endDate),
    [range.startDate, range.endDate],
  );
  const buckets = useMemo(
    () => (hourly ? calendarHours(range.startDate, range.endDate) : days),
    [hourly, range.startDate, range.endDate, days],
  );
  const aligned: Array<UsagePoint | undefined> = useMemo(
    () =>
      hours === null
        ? alignToBuckets(range.days, buckets, (point) => point.date)
        : alignToBuckets(hours.hours, buckets, (point) => point.hourStart),
    [hours, range.days, buckets],
  );
  const resolution = hourly ? "hour" : "day";
  // Opening a bucket only means something where the breakdown below is keyed the same way,
  // which is by day. An hourly chart therefore reads rather than selects.
  const daySelection = hourly ? {} : { selectedBucket: openDay, onSelectBucket: setOpenDay };

  const openPoint: DailyUsagePoint | null =
    openDay === null ? null : (range.days.find((day) => day.date === openDay) ?? null);
  const usage = openPoint?.usage ?? range.usage;
  const models = openPoint?.models ?? range.models;
  const cost = openPoint ? openPoint.apiEquivalentCostUsd : range.apiEquivalentCostUsd;

  const activeDayAverage =
    range.days.length === 0 ? 0 : Math.round(range.usage.total / range.days.length);
  const previousActiveAverage =
    previousRange === null || previousRange.days.length === 0
      ? 0
      : Math.round(previousRange.usage.total / previousRange.days.length);
  const customTooLong = customRangeTooLong(customStart, customEnd);
  const customInvalid = !customStart || !customEnd || customStart > customEnd || customTooLong;
  const displayDays = [...range.days].reverse();

  const tokenSeries: ChartSeries[] = CATEGORIES.map((category) => ({
    key: category.key,
    label: category.label,
    color: category.color,
    values: aligned.map((point) => (point ? point.usage[category.key] : 0)),
  }));

  const costSeries: ChartSeries[] = [
    {
      key: "cost",
      label: "API-equivalent cost",
      color: SERIES_SLOTS[0],
      values: aligned.map((point) => point?.apiEquivalentCostUsd ?? 0),
    },
  ];

  const chartModels = namedModels(range.models);
  const modelSeries: ChartSeries[] = chartModels.map((model, index) => ({
    key: model,
    label: model,
    color: SERIES_SLOTS[index],
    values: aligned.map(
      (point) => point?.models.find((entry) => entry.model === model)?.tokens ?? 0,
    ),
  }));
  if (range.models.length > chartModels.length) {
    modelSeries.push({
      key: "other-models",
      label: `Other (${range.models.length - chartModels.length})`,
      color: SERIES_REST,
      values: aligned.map((point) =>
        (point?.models ?? [])
          .filter((entry) => !chartModels.includes(entry.model))
          .reduce((sum, entry) => sum + entry.tokens, 0),
      ),
    });
  }

  const quotaSeries: ChartSeries[] = (quotaHistory?.windows ?? []).map((window, index) => ({
    key: window.kind,
    label: window.label,
    color: SERIES_SLOTS[index],
    values: days.map(
      (day) => window.points.find((point) => point.date === day)?.peakUsedPercent ?? null,
    ),
  }));
  const quotaMarkers: ChartMarker[] = (quotaHistory?.resets ?? []).map((reset) => ({
    bucket: toLocalDateString(new Date(reset.anchoredAt * 1_000)),
    label: `${reset.windowLabel} restarted (${reset.classification})`,
    tone: reset.classification === "unplanned" ? "warning" : "muted",
  }));

  function applyPreset(preset: Exclude<RangePreset, "custom">) {
    setShowCustom(false);
    setOpenDay(null);
    onSelectRange(createPresetRange(preset));
  }

  function applyCustom() {
    if (customInvalid) return;
    setOpenDay(null);
    onSelectRange(createCustomRange(customStart, customEnd));
    setShowCustom(false);
  }

  function toggleCustom() {
    if (!showCustom) {
      setCustomStart(selection.startDate);
      setCustomEnd(selection.endDate);
    }
    setShowCustom((current) => !current);
  }

  return (
    <section
      className="history-section"
      aria-label={`${combined ? "Combined" : snapshot.displayName} usage history`}
    >
      <div className="history-heading">
        <div>
          <span className="section-kicker">Usage history</span>
          <h2>{selection.label}</h2>
        </div>
        {providers.length > 1 || range.devices.length > 1 ? (
          <div className="history-filters">
            {providers.length > 1 ? (
              <div className="provider-tabs" role="tablist" aria-label="Usage history provider">
                {/* Everything counted together comes first, because it is the whole and
                    each provider below it is one part of that total. */}
                <button
                  type="button"
                  role="tab"
                  aria-selected={combined}
                  className={combined ? "active" : ""}
                  onClick={() => {
                    setOpenDay(null);
                    onSelectProvider("all");
                  }}
                >
                  All
                </button>
                {providers.map((provider) => (
                  <button
                    type="button"
                    key={provider.provider}
                    role="tab"
                    aria-selected={provider.provider === activeProvider}
                    className={provider.provider === activeProvider ? "active" : ""}
                    onClick={() => {
                      setOpenDay(null);
                      onSelectProvider(provider.provider);
                    }}
                  >
                    {provider.displayName}
                  </button>
                ))}
              </div>
            ) : null}
            {range.devices.length > 1 ? (
              <label className="device-filter">
                <span>Device</span>
                <select
                  value={activeDevice ?? ""}
                  disabled={loading}
                  onChange={(event) => {
                    setOpenDay(null);
                    onSelectDevice(event.target.value === "" ? null : event.target.value);
                  }}
                >
                  <option value="">All devices</option>
                  {range.devices.map((device) => (
                    <option key={device.deviceId} value={device.deviceId}>
                      {device.local ? "This machine" : device.displayName}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
          </div>
        ) : null}
        <div className="range-control" role="group" aria-label="Usage date range">
          {PRESETS.map((preset) => (
            <button
              type="button"
              key={preset.value}
              className={selection.preset === preset.value ? "active" : ""}
              onClick={() => applyPreset(preset.value)}
              disabled={loading}
              aria-pressed={selection.preset === preset.value}
            >
              {preset.label}
            </button>
          ))}
          <button
            type="button"
            className={selection.preset === "custom" || showCustom ? "active" : ""}
            onClick={toggleCustom}
            disabled={loading}
            aria-expanded={showCustom}
          >
            <CalendarDays aria-hidden="true" /> Custom
          </button>
        </div>
      </div>

      {showCustom ? (
        <div className="custom-range-panel">
          <label>
            From
            <input
              type="date"
              value={customStart}
              max={customEnd || todayString()}
              onChange={(event) => setCustomStart(event.target.value)}
            />
          </label>
          <label>
            To
            <input
              type="date"
              value={customEnd}
              min={customStart}
              max={todayString()}
              onChange={(event) => setCustomEnd(event.target.value)}
            />
          </label>
          {customTooLong ? (
            <span className="custom-range-error">
              Choose no more than {MAX_CUSTOM_RANGE_DAYS} days.
            </span>
          ) : null}
          <button type="button" onClick={applyCustom} disabled={customInvalid || loading}>
            Apply range
          </button>
        </div>
      ) : null}

      {error ? <p className="range-error">Unable to load this range: {error}</p> : null}

      <div className="history-content">
        {/* The model count is not a fourth headline figure: the model mix card below both
            counts them and says what they were. Each figure carries how it moved against
            the period of the same length before this one, so a total means something on
            its own. */}
        <div className="summary-strip">
          <StatTile
            label="Total tokens"
            value={formatNumber(range.usage.total)}
            delta={previousRange && formatDelta(range.usage.total, previousRange.usage.total)}
          />
          <StatTile
            label="API-equivalent cost"
            value={formatCurrency(range.apiEquivalentCostUsd)}
            delta={
              previousRange &&
              formatDelta(range.apiEquivalentCostUsd ?? 0, previousRange.apiEquivalentCostUsd ?? 0)
            }
          />
          <StatTile
            label="Active-day average"
            value={formatNumber(activeDayAverage)}
            delta={previousRange && formatDelta(activeDayAverage, previousActiveAverage)}
          />
        </div>

        {/* One day is not a trend: a single column would be a bar chart of one, and the
            figures above and the breakdown below already say everything it could. An
            hourly range is never in that position — a day is twenty-four buckets. */}
        {buckets.length < 2 ? (
          <p className="chart-hint">
            Charts compare one period against another. Choose a longer range to see them.
          </p>
        ) : null}
        <div className="chart-grid" hidden={buckets.length < 2}>
          <TrendChart
            title={hourly ? "Hourly tokens" : "Daily tokens"}
            subtitle={
              hourly
                ? "Stacked by category · one column per hour"
                : "Stacked by category · select a day to open it below"
            }
            buckets={buckets}
            resolution={resolution}
            series={tokenSeries}
            mode="stacked"
            formatValue={formatNumber}
            formatTick={formatCompactNumber}
            {...daySelection}
            emptyCopy="No usage recorded in this date range."
            loading={loading}
          />
          <TrendChart
            title="Cost trend"
            subtitle={`Estimated API-equivalent cost per ${resolution}`}
            buckets={buckets}
            resolution={resolution}
            series={costSeries}
            mode="line"
            formatValue={(value) => formatCurrency(value)}
            formatTick={formatCompactCurrency}
            emptyCopy="No cost recorded in this date range."
            loading={loading}
          />
          <TrendChart
            title="Model trend"
            subtitle={`${range.models.length} models · by tokens per ${resolution}`}
            buckets={buckets}
            resolution={resolution}
            series={modelSeries}
            mode="stacked"
            formatValue={formatNumber}
            formatTick={formatCompactNumber}
            {...daySelection}
            emptyCopy="No model usage recorded in this date range."
            loading={loading}
          />
          {/* Quota is measured once a poll rather than once a request, and a day is
              summarised by its peak, so this chart stays on the daily axis whatever
              resolution the usage beside it is read at. */}
          {quotaSeries.length > 0 ? (
            <TrendChart
              title="Quota history"
              subtitle="Highest share of each window used that day"
              buckets={days}
              resolution="day"
              series={quotaSeries}
              mode="line"
              maxValue={100}
              formatValue={(value) => `${value.toFixed(1)}%`}
              formatTick={(value) => `${value}%`}
              markers={quotaMarkers}
              emptyCopy="No quota readings were recorded in this date range."
              loading={loading}
            />
          ) : null}
        </div>

        {openPoint ? (
          <div className="day-drilldown">
            <span>
              Showing <strong>{formatRangeDate(openPoint.date)}</strong> —{" "}
              {formatNumber(openPoint.usage.total)} tokens across {openPoint.models.length} model
              {openPoint.models.length === 1 ? "" : "s"}
            </span>
            <button type="button" onClick={() => setOpenDay(null)}>
              <X aria-hidden="true" /> Back to the range
            </button>
          </div>
        ) : null}

        <div className={`history-grid${range.devices.length > 1 ? " multi-device" : ""}`}>
          <article className="history-card breakdown-card">
            <div className="card-heading">
              <div>
                <h3>Daily breakdown</h3>
                <span>Newest first · {range.days.length} active days</span>
              </div>
              <span>Tokens and estimated API cost</span>
            </div>
            {range.days.length === 0 ? (
              <p className="empty-copy">No usage recorded in this date range.</p>
            ) : (
              <div className="daily-table" role="table" aria-label="Daily token and cost breakdown">
                <div className="daily-table-row daily-table-header" role="row">
                  <span role="columnheader">Date</span>
                  <span role="columnheader">Total tokens</span>
                  <span role="columnheader">Cached input</span>
                  <span role="columnheader">Output</span>
                  <span role="columnheader">API cost</span>
                </div>
                {displayDays.map((day) => (
                  <div
                    className={`daily-table-row${day.date === openDay ? " open" : ""}`}
                    role="row"
                    key={day.date}
                    tabIndex={0}
                    onClick={() => setOpenDay(day.date === openDay ? null : day.date)}
                    onKeyDown={(event) => {
                      if (event.key !== "Enter" && event.key !== " ") return;
                      event.preventDefault();
                      setOpenDay(day.date === openDay ? null : day.date);
                    }}
                  >
                    <strong role="cell">{formatRangeDate(day.date)}</strong>
                    <span role="cell">{formatNumber(day.usage.total)}</span>
                    <span role="cell">{formatNumber(day.usage.cacheRead)}</span>
                    <span role="cell">{formatNumber(day.usage.output)}</span>
                    <span role="cell">{formatCurrency(day.apiEquivalentCostUsd)}</span>
                  </div>
                ))}
              </div>
            )}
          </article>

          <article className="history-card model-card">
            <div className="card-heading">
              <div>
                <h3>Model mix</h3>
                <span>{models.length} models · by total tokens</span>
              </div>
              {openPoint ? <span>{formatRangeDate(openPoint.date)}</span> : null}
            </div>
            <div className="model-list">
              {models.length === 0 ? (
                <p className="empty-copy">No model usage recorded.</p>
              ) : (
                models.slice(0, 6).map((model) => (
                  <div className="model-row" key={model.model}>
                    <span title={model.model}>{model.model}</span>
                    <span>{model.percent.toFixed(1)}%</span>
                    <div className="mini-track">
                      <i style={{ width: `${model.percent}%` }} />
                    </div>
                  </div>
                ))
              )}
            </div>
          </article>

          {range.devices.length > 1 ? (
            <article className="history-card device-card">
              <div className="card-heading">
                <div>
                  <h3>Devices</h3>
                  <span>{range.devices.length} devices · by total tokens</span>
                </div>
              </div>
              <div className="device-list">
                {range.devices.map((device) => (
                  <div className="device-row" key={device.deviceId}>
                    <span title={device.displayName}>
                      {device.displayName}
                      {device.local ? <small>This machine</small> : null}
                    </span>
                    <strong>{formatNumber(device.tokens)}</strong>
                    <span>{device.percent.toFixed(1)}%</span>
                    <div className="device-track">
                      <i style={{ width: `${device.percent}%` }} />
                    </div>
                  </div>
                ))}
              </div>
            </article>
          ) : null}

          <article className="history-card token-card">
            <div className="card-heading">
              <div>
                <h3>Token breakdown</h3>
                <span>
                  {combined
                    ? `${providers.length} providers combined`
                    : `Catalog ${formatRevision(snapshot.pricingCatalogRevision)}`}
                </span>
              </div>
              <span>
                {openPoint
                  ? formatCompactCurrency(cost ?? 0)
                  : combined
                    ? null
                    : (snapshot.planType ?? "Unknown plan")}
              </span>
            </div>
            <div className="token-list">
              {CATEGORIES.map((category) => (
                <div className="token-row" key={category.key}>
                  <i className="token-dot" style={{ background: category.color }} />
                  <span>{category.label}</span>
                  <strong>{formatNumber(usage[category.key])}</strong>
                  <span>
                    {usage.total === 0
                      ? "0.0"
                      : ((usage[category.key] / usage.total) * 100).toFixed(1)}
                    %
                  </span>
                </div>
              ))}
            </div>
          </article>
        </div>
      </div>
    </section>
  );
}

/** A headline figure with how it moved against the period before it. */
function StatTile({
  label,
  value,
  delta,
}: {
  label: string;
  value: string;
  delta: string | null | undefined;
}) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
      {/* Neither direction is good or bad here — more usage is not a failure and less is
          not a win — so the change is stated rather than coloured. */}
      {delta ? <em className="delta">{delta} vs previous period</em> : null}
    </div>
  );
}
