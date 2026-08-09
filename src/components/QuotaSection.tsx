import type { LimitWindow } from "../types";
import { formatCountdown } from "../format";

interface QuotaSectionProps {
  limits: LimitWindow[];
  earnedResetCount: number | null;
}

function QuotaRow({ limit }: { limit: LimitWindow }) {
  const used = limit.usedPercent;
  return (
    <div className="quota-row">
      <div className="quota-label">
        <h2>{limit.label}</h2>
        <p>
          {limit.windowDurationMins === null
            ? "Window duration unavailable"
            : `${Math.round(limit.windowDurationMins / 60)} hour quota window`}
        </p>
      </div>
      <div className="quota-meter" aria-label={`${limit.label} usage`}>
        <div className="quota-track">
          <span style={{ width: `${Math.min(100, Math.max(0, used ?? 0))}%` }} />
        </div>
      </div>
      <div className="quota-percent">
        <strong>{used === null ? "—" : `${used.toFixed(1)}%`}</strong>
        <span>{used === null ? "unavailable" : "used"}</span>
      </div>
      <div className="quota-reset">
        <span>Resets in</span>
        <strong>{formatCountdown(limit.resetsAt)}</strong>
      </div>
    </div>
  );
}

export function QuotaSection({ limits, earnedResetCount }: QuotaSectionProps) {
  return (
    <section className="quota-section" aria-label="Codex quota windows">
      {limits.length > 0 ? (
        limits.map((limit) => <QuotaRow key={limit.kind} limit={limit} />)
      ) : (
        <div className="quota-empty">
          <h2>Quota windows unavailable</h2>
          <p>QuotaStation will keep retrying the local Codex read interface.</p>
        </div>
      )}
      <div className="reset-inventory">
        <span>Earned resets</span>
        <strong>{earnedResetCount ?? "Unknown"}</strong>
      </div>
    </section>
  );
}
