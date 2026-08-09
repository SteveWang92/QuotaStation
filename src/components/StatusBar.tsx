import { AlertTriangle, CheckCircle2 } from "lucide-react";
import { formatTimestamp } from "../format";
import type { ProviderSnapshot } from "../types";

export function StatusBar({ snapshot }: { snapshot: ProviderSnapshot }) {
  const healthy = snapshot.freshness === "fresh" && !snapshot.liveError && !snapshot.historyError;
  const message = snapshot.liveError ?? snapshot.historyError ?? "Codex quota and local history are current.";
  return (
    <footer className={`status-bar ${healthy ? "healthy" : "attention"}`}>
      {healthy ? <CheckCircle2 aria-hidden="true" /> : <AlertTriangle aria-hidden="true" />}
      <strong>{healthy ? "Data current" : snapshot.freshness === "stale" ? "Stale data" : "Provider unavailable"}</strong>
      <span className="status-message">{message}</span>
      <span className="last-success">Last success {formatTimestamp(snapshot.lastSuccessAt)}</span>
    </footer>
  );
}
