import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { formatCompactCountdown, formatWindowBadge } from "../format";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";

/** The taskbar has no room for a full provider name beside the window badge. */
const SHORT_NAME: Record<string, string> = { codex: "CDX", claude: "CLD" };

function shortName(snapshot: ProviderSnapshot) {
  return SHORT_NAME[snapshot.provider] ?? snapshot.displayName.slice(0, 3).toUpperCase();
}

function ProviderColumn({ snapshot }: { snapshot: ProviderSnapshot }) {
  const badge = shortName(snapshot);
  const statusColor = snapshot.compactStatus.color;
  return (
    <div className="taskbar-provider">
      {/* One name for the whole column rather than one per row: a provider showing both of
          its windows was repeating its own name, which is the widest thing in a slot that
          has 40px of taskbar to work with. */}
      <span className="taskbar-name" style={{ color: statusColor }}>
        {badge}
      </span>
      <div className="taskbar-windows">
      {snapshot.limits.length > 0 ? (
        snapshot.limits.map((limit) => (
          <div className="taskbar-quota" key={limit.kind}>
            <span>{formatWindowBadge(limit.windowDurationMins, limit.label)}</span>
            <strong
              style={{ color: statusColor }}
              aria-label={
                limit.remainingPercent === null
                  ? `${snapshot.displayName} ${limit.label}: remaining percentage unavailable`
                  : undefined
              }
            >
              {limit.remainingPercent === null ? "—" : `${Math.round(limit.remainingPercent)}%`}
            </strong>
            <time dateTime={limit.resetsAt === null ? undefined : new Date(limit.resetsAt * 1_000).toISOString()}>
              {formatCompactCountdown(limit.resetsAt)}
            </time>
            {/* The bar fills with what has been consumed, exactly as the dashboard's meter
                does. Filling it with what is left would leave the same quota drawn one way
                in the taskbar and the other way in the window above it. */}
            {limit.usedPercent === null ? (
              <i className="unknown" aria-hidden="true" />
            ) : (
              <i aria-label={`${snapshot.displayName} ${limit.label}: ${limit.usedPercent}% used`}>
                <b
                  style={{
                    width: `${Math.min(100, Math.max(0, limit.usedPercent))}%`,
                    background: statusColor,
                  }}
                />
              </i>
            )}
          </div>
        ))
      ) : (
        <span className="taskbar-unavailable" style={{ color: statusColor }}>
          unavailable
        </span>
      )}
      </div>
    </div>
  );
}

export function TaskbarWidget({ initialWorkspace }: { initialWorkspace: WorkspaceSnapshot }) {
  const { workspace } = useSnapshot(initialWorkspace);
  const providers = workspace.providers;

  useEffect(() => {
    // The widget holds one size whatever it is showing, so this only re-docks the window
    // after the renderer mounts. The taskbar slice has no room for an error surface; the
    // tray icon and the dashboard remain the recovery surfaces for a failed placement.
    void invoke("set_taskbar_widget_size").catch(() => {});
  }, []);

  return (
    <main
      className="taskbar-widget-shell"
      style={{ "--taskbar-status-color": workspace.aggregate.color } as React.CSSProperties}
    >
      {providers.length > 0 ? (
        providers.map((snapshot) => <ProviderColumn key={snapshot.provider} snapshot={snapshot} />)
      ) : (
        <span className="taskbar-unavailable">No provider detected</span>
      )}
    </main>
  );
}
