import { AlertTriangle, CheckCircle2, ChevronDown } from "lucide-react";
import { useState } from "react";
import { formatTimestamp } from "../format";
import type { DiagnosticsSnapshot, ProviderSnapshot } from "../types";

export function StatusBar({ snapshot, diagnostics }: { snapshot: ProviderSnapshot; diagnostics: DiagnosticsSnapshot }) {
  const [expanded, setExpanded] = useState(false);
  const status = snapshot.compactStatus;
  const healthy = status.level === "healthy";
  return (
    <footer className={`status-bar ${status.level}`} style={{ "--status-color": status.color } as React.CSSProperties}>
      <div className="status-summary">
        {healthy ? <CheckCircle2 aria-hidden="true" /> : <AlertTriangle aria-hidden="true" />}
        <strong>{status.label}</strong>
        <span className="status-message">{status.message}</span>
        <button
          type="button"
          className="diagnostics-toggle"
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          Diagnostics
          <ChevronDown aria-hidden="true" className={expanded ? "expanded" : ""} />
        </button>
      </div>
      {expanded ? (
        <div className="diagnostics-panel">
          <div className="diagnostics-grid">
            {diagnostics.acquisitions.map((acquisition) => (
              <div className="diagnostic-item" key={acquisition.acquisitionPath}>
                <span>{acquisition.label}</span>
                <strong className={acquisition.status}>{acquisition.status}</strong>
                <small>
                  {acquisition.status === "succeeded" ? "Updated" : "Last attempt"}{" "}
                  {formatTimestamp(acquisition.status === "succeeded" ? acquisition.lastSuccessAt : acquisition.lastAttemptAt)}
                </small>
                {acquisition.error ? <small className="diagnostic-error">{acquisition.error}</small> : null}
              </div>
            ))}
            <div className="diagnostic-item">
              <span>Data retention</span>
              <strong className={diagnostics.retention.status}>{diagnostics.retention.status}</strong>
              <small>Last completed {formatTimestamp(diagnostics.retention.lastCompletedAt)}</small>
              {diagnostics.retention.error ? <small className="diagnostic-error">{diagnostics.retention.error}</small> : null}
            </div>
            <div className="diagnostic-item">
              <span>Session watcher</span>
              <strong className={diagnostics.watcher.status === "active" ? "succeeded" : "failed"}>
                {diagnostics.watcher.status}
              </strong>
              <small>{diagnostics.watcher.watchedLocationCount} local locations · Last event {formatTimestamp(diagnostics.watcher.lastEventAt)}</small>
              {diagnostics.watcher.error ? <small className="diagnostic-error">{diagnostics.watcher.error}</small> : null}
            </div>
          </div>
          <div className="diagnostic-revisions">
            <span>ccusage <code>{diagnostics.parserRevision.slice(0, 12)}</code></span>
            <span>Pricing <code>{diagnostics.pricingCatalogRevision.slice(0, 12)}</code></span>
          </div>
        </div>
      ) : null}
    </footer>
  );
}
