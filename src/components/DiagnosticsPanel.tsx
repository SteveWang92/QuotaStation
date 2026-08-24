import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { formatResetTimestamp, formatRevision, formatTimestamp } from "../format";
import type { DiagnosticsSnapshot, LimitWindow, ProviderSnapshot } from "../types";

interface DiagnosticsPanelProps {
  diagnostics: DiagnosticsSnapshot;
  /** The providers on screen, so each quota window can say where its reading came from. */
  providers: ProviderSnapshot[];
  interfaceError: string | null;
}

const SOURCE_LABELS: Record<LimitWindow["source"], string> = {
  app_server: "App server",
  session_log: "Session log",
  status_line: "Status line",
};

/**
 * Every acquisition path reports separately, because they fail separately: a provider that
 * answered and a watcher that stopped are two different problems, and the panel is where
 * the difference is visible. It shows no full paths and no session content.
 */
export function DiagnosticsPanel({
  diagnostics,
  providers,
  interfaceError,
}: DiagnosticsPanelProps) {
  // A built application has no console, and the status-line bridge is a process that lives
  // for milliseconds inside Claude Code. The log is where both of them report, so the panel
  // has to be able to point at it.
  const [logAvailable, setLogAvailable] = useState(false);

  useEffect(() => {
    void invoke<boolean>("get_log_available").then(setLogAvailable);
  }, []);

  return (
    <div className="diagnostics-panel">
      <div className="diagnostics-grid">
        {diagnostics.acquisitions.map((acquisition) => (
          <div className="diagnostic-item" key={acquisition.acquisitionPath}>
            <span>{acquisition.label}</span>
            <strong className={acquisition.status}>{acquisition.status}</strong>
            <small>
              {acquisition.status === "succeeded" ? "Updated" : "Last attempt"}{" "}
              {formatTimestamp(
                acquisition.status === "succeeded"
                  ? acquisition.lastSuccessAt
                  : acquisition.lastAttemptAt,
              )}
            </small>
            {acquisition.error ? (
              <small className="diagnostic-error">{acquisition.error}</small>
            ) : null}
          </div>
        ))}
        <div className="diagnostic-item">
          <span>Data retention</span>
          <strong className={diagnostics.retention.status}>{diagnostics.retention.status}</strong>
          <small>Last completed {formatTimestamp(diagnostics.retention.lastCompletedAt)}</small>
          {diagnostics.retention.error ? (
            <small className="diagnostic-error">{diagnostics.retention.error}</small>
          ) : null}
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
          <small>
            {diagnostics.watcher.watchedLocationCount} local locations · Last event{" "}
            {formatTimestamp(diagnostics.watcher.lastEventAt)}
          </small>
          {diagnostics.watcher.error ? (
            <small className="diagnostic-error">{diagnostics.watcher.error}</small>
          ) : null}
        </div>
      </div>
      {/* The quota rows themselves show the numbers and nothing about where they came
          from. A window read from a session log is as current as the last session, one read
          from a status line as current as the last turn, so which source produced it is
          what explains an unexpected reading — and that belongs here. */}
      {providers.some((provider) => provider.limits.length > 0) ? (
        <div className="diagnostic-sources">
          {providers.flatMap((provider) =>
            provider.limits.map((limit) => (
              <span key={`${provider.provider}-${limit.kind}`}>
                {provider.displayName} {limit.label.toLowerCase()} · {SOURCE_LABELS[limit.source]} ·
                as of {formatResetTimestamp(limit.observedAt)}
              </span>
            )),
          )}
        </div>
      ) : null}
      <div className="diagnostic-revisions">
        <span>
          ccusage <code>{formatRevision(diagnostics.parserRevision)}</code>
        </span>
        <span>
          Pricing <code>{formatRevision(diagnostics.pricingCatalogRevision)}</code>
        </span>
        <span>
          QuotaStation{" "}
          <code>
            {diagnostics.appVersion} ({diagnostics.buildCommit})
          </code>{" "}
          {diagnostics.buildKind}
        </span>
        {logAvailable ? (
          <button
            type="button"
            className="diagnostic-log"
            onClick={() => void invoke("reveal_log_file")}
          >
            Show activity log
          </button>
        ) : null}
      </div>
    </div>
  );
}
