import { AlertTriangle, CheckCircle2, ChevronDown } from "lucide-react";
import { useState } from "react";
import { formatTimestamp } from "../format";
import type { DiagnosticsSnapshot, ProviderSnapshot } from "../types";

export function StatusBar({ snapshot, diagnostics }: { snapshot: ProviderSnapshot; diagnostics: DiagnosticsSnapshot }) {
  const [expanded, setExpanded] = useState(false);
  const healthy = snapshot.freshness === "fresh" && !snapshot.liveError && !snapshot.historyError;
  const message = snapshot.liveError ?? snapshot.historyError ?? "Codex quota and local history are current.";
  return (
    <footer className={`status-bar ${healthy ? "healthy" : "attention"}`}>
      <div className="status-summary">
        {healthy ? <CheckCircle2 aria-hidden="true" /> : <AlertTriangle aria-hidden="true" />}
        <strong>{healthy ? "Data current" : snapshot.freshness === "stale" ? "Stale data" : "Provider unavailable"}</strong>
        <span className="status-message">{message}</span>
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
