import { CalendarDays } from "lucide-react";
import { useState } from "react";
import {
  createCustomRange,
  createPresetRange,
  formatRangeDate,
  todayString,
  type DateRangeSelection,
  type RangePreset,
} from "../dateRanges";
import { formatCurrency, formatNumber, formatRevision } from "../format";
import type { ProviderKey, ProviderSnapshot, UsageRangeSnapshot } from "../types";

const PRESETS: Array<{ value: Exclude<RangePreset, "custom">; label: string }> = [
  { value: "today", label: "Today" },
  { value: "3d", label: "3d" },
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
];

interface UsageSummaryProps {
  snapshot: ProviderSnapshot;
  /** All enabled providers, so the history can be switched between them. */
  providers: ProviderSnapshot[];
  activeProvider: ProviderKey;
  onSelectProvider: (provider: ProviderKey) => void;
  range: UsageRangeSnapshot;
  selection: DateRangeSelection;
  loading: boolean;
  error: string | null;
  onSelectRange: (range: DateRangeSelection) => void;
}

export function UsageSummary({
  snapshot,
  providers,
  activeProvider,
  onSelectProvider,
  range,
  selection,
  loading,
  error,
  onSelectRange,
}: UsageSummaryProps) {
  const [showCustom, setShowCustom] = useState(false);
  const [customStart, setCustomStart] = useState(selection.startDate);
  const [customEnd, setCustomEnd] = useState(selection.endDate);
  const { usage } = range;
  const categories = [
    ["Input", usage.input, "lime"],
    ["Output", usage.output, "violet"],
    ["Cached input", usage.cacheRead, "blue"],
    ["Reasoning", usage.reasoning, "muted"],
  ] as const;
  const activeDayAverage = range.days.length === 0 ? 0 : Math.round(usage.total / range.days.length);
  const customInvalid = !customStart || !customEnd || customStart > customEnd;
  const displayDays = [...range.days].reverse();

  function applyPreset(preset: Exclude<RangePreset, "custom">) {
    setShowCustom(false);
    onSelectRange(createPresetRange(preset));
  }

  function applyCustom() {
    if (customInvalid) return;
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
    <section className="history-section" aria-label={`${snapshot.displayName} usage history`}>
      <div className="history-heading">
        <div>
          <span className="section-kicker">Usage history</span>
          <h2>{selection.label}</h2>
        </div>
        {providers.length > 1 ? (
          <div className="provider-tabs" role="tablist" aria-label="Usage history provider">
            {providers.map((provider) => (
              <button
                type="button"
                key={provider.provider}
                role="tab"
                aria-selected={provider.provider === activeProvider}
                className={provider.provider === activeProvider ? "active" : ""}
                onClick={() => onSelectProvider(provider.provider)}
              >
                {provider.displayName}
              </button>
            ))}
          </div>
        ) : null}
        <div className="range-control" aria-label="Usage date range">
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
            <input type="date" value={customStart} max={customEnd || todayString()} onChange={(event) => setCustomStart(event.target.value)} />
          </label>
          <label>
            To
            <input type="date" value={customEnd} min={customStart} max={todayString()} onChange={(event) => setCustomEnd(event.target.value)} />
          </label>
          <button type="button" onClick={applyCustom} disabled={customInvalid || loading}>Apply range</button>
        </div>
      ) : null}

      {error ? <p className="range-error">Unable to load this range: {error}</p> : null}

      <div className="history-content">
        {/* The model count is not a fourth headline figure: the model mix card below both
            counts them and says what they were. */}
        <div className="summary-strip">
          <div><span>Total tokens</span><strong>{formatNumber(usage.total)}</strong></div>
          <div><span>API-equivalent cost</span><strong>{formatCurrency(range.apiEquivalentCostUsd)}</strong></div>
          <div><span>Active-day average</span><strong>{formatNumber(activeDayAverage)}</strong></div>
        </div>

        <div className="history-grid">
          <article className="history-card breakdown-card">
            <div className="card-heading">
              <div><h3>Daily breakdown</h3><span>Newest first · {range.days.length} active days</span></div>
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
                  <div className="daily-table-row" role="row" key={day.date}>
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
              <div><h3>Model mix</h3><span>{range.models.length} models · by total tokens</span></div>
            </div>
            <div className="model-list">
              {range.models.length === 0 ? (
                <p className="empty-copy">No model usage recorded.</p>
              ) : range.models.slice(0, 6).map((model) => (
                <div className="model-row" key={model.model}>
                  <span title={model.model}>{model.model}</span>
                  <span>{model.percent.toFixed(1)}%</span>
                  <div className="mini-track"><i style={{ width: `${model.percent}%` }} /></div>
                </div>
              ))}
            </div>
          </article>

          <article className="history-card token-card">
            <div className="card-heading">
              <div><h3>Token breakdown</h3><span>Catalog {formatRevision(snapshot.pricingCatalogRevision)}</span></div>
              <span>{snapshot.planType ?? "Unknown plan"}</span>
            </div>
            <div className="token-list">
              {categories.map(([label, value, tone]) => (
                <div className="token-row" key={label}>
                  <i className={`token-dot ${tone}`} />
                  <span>{label}</span>
                  <strong>{formatNumber(value)}</strong>
                  <span>{usage.total === 0 ? "0.0" : ((value / usage.total) * 100).toFixed(1)}%</span>
                </div>
              ))}
            </div>
          </article>
        </div>
      </div>
    </section>
  );
}
