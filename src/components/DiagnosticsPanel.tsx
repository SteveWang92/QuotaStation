import { formatRevision, formatTimestamp } from "../format";
import type { DiagnosticsSnapshot } from "../types";

interface DiagnosticsPanelProps {
  diagnostics: DiagnosticsSnapshot;
  interfaceError: string | null;
}

/**
 * Every acquisition path reports separately, because they fail separately: a provider that
 * answered and a watcher that stopped are two different problems, and the panel is where
 * the difference is visible. It shows no full paths and no session content.
 */
export function DiagnosticsPanel({ diagnostics, interfaceError }: DiagnosticsPanelProps) {
  return (
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
          <span>Application interface</span>
          <strong className={interfaceError ? "failed" : "succeeded"}>
            {interfaceError ? "failed" : "connected"}
          </strong>
          <small>Local command channel to the QuotaStation core</small>
          {interfaceError ? <small className="diagnostic-error">{interfaceError}</small> : null}
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
        <span>ccusage <code>{formatRevision(diagnostics.parserRevision)}</code></span>
        <span>Pricing <code>{formatRevision(diagnostics.pricingCatalogRevision)}</code></span>
      </div>
    </div>
  );
}
