import { formatCurrency, formatNumber } from "../format";
import type { ProviderSnapshot } from "../types";

export function UsageSummary({ snapshot }: { snapshot: ProviderSnapshot }) {
  const { today } = snapshot;
  const categories = [
    ["Input", today.input, "lime"],
    ["Output", today.output, "violet"],
    ["Cached input", today.cacheRead, "blue"],
    ["Reasoning", today.reasoning, "muted"],
  ] as const;

  return (
    <section className="usage-grid" aria-label="Today's Codex usage">
      <div className="usage-column">
        <h2>Today</h2>
        <span className="eyeline">Total tokens</span>
        <strong className="metric-value">{formatNumber(today.total)}</strong>
        <dl className="metric-list">
          <div><dt>Plan</dt><dd>{snapshot.planType ?? "Unknown"}</dd></div>
          <div><dt>Source</dt><dd>Local Codex sessions</dd></div>
        </dl>
      </div>
      <div className="usage-column">
        <h2>API-equivalent cost</h2>
        <strong className="metric-value">{formatCurrency(snapshot.apiEquivalentCostUsd)}</strong>
        <span className="metric-note">USD · catalog {snapshot.pricingCatalogRevision.slice(0, 12)}</span>
        <dl className="metric-list compact">
          <div><dt>Estimate only</dt><dd>Not a bill</dd></div>
          <div><dt>Service tier</dt><dd>Recorded or standard</dd></div>
        </dl>
      </div>
      <div className="usage-column">
        <h2>Model mix</h2>
        <div className="model-list">
          {snapshot.models.length === 0 ? (
            <p className="empty-copy">No model usage recorded today.</p>
          ) : snapshot.models.slice(0, 4).map((model) => (
            <div className="model-row" key={model.model}>
              <span>{model.model}</span>
              <span>{model.percent.toFixed(1)}%</span>
              <div className="mini-track"><i style={{ width: `${model.percent}%` }} /></div>
            </div>
          ))}
        </div>
      </div>
      <div className="usage-column token-column">
        <h2>Token breakdown</h2>
        <div className="token-list">
          {categories.map(([label, value, tone]) => (
            <div className="token-row" key={label}>
              <i className={`token-dot ${tone}`} />
              <span>{label}</span>
              <strong>{formatNumber(value)}</strong>
              <span>{today.total === 0 ? "0.0" : ((value / today.total) * 100).toFixed(1)}%</span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
