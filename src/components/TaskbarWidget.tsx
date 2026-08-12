import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { formatCompactCountdown, formatWindowBadge } from "../format";
import type { WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";

export function TaskbarWidget({ initialWorkspace }: { initialWorkspace: WorkspaceSnapshot }) {
  const { workspace } = useSnapshot(initialWorkspace);
  const snapshot = workspace.providers[0];

  useEffect(() => {
    // The taskbar slice has no room for an error surface; the tray icon and the
    // dashboard remain the recovery surfaces for a failed resize.
    void invoke("set_taskbar_widget_columns", { columns: Math.max(1, snapshot.limits.length) }).catch(() => {});
  }, [snapshot.limits.length]);

  return (
    <main className={`taskbar-widget-shell${snapshot.limits.length <= 1 ? " single" : ""}`} style={{ "--taskbar-status-color": workspace.aggregate.color } as React.CSSProperties}>
      {snapshot.limits.length > 0 ? snapshot.limits.map((limit) => (
        <div className="taskbar-quota" key={limit.kind}>
          <span>{formatWindowBadge(limit.windowDurationMins, limit.label)}</span>
          <strong>{limit.remainingPercent === null ? "—" : `${Math.round(limit.remainingPercent)}%`}</strong>
          <time dateTime={limit.resetsAt === null ? undefined : new Date(limit.resetsAt * 1_000).toISOString()}>
            {formatCompactCountdown(limit.resetsAt)}
          </time>
          <i aria-label={`${limit.remainingPercent ?? 0}% remaining`}>
            <b style={{ width: `${limit.remainingPercent ?? 0}%` }} />
          </i>
        </div>
      )) : <span className="taskbar-unavailable">{snapshot.displayName} unavailable</span>}
    </main>
  );
}
