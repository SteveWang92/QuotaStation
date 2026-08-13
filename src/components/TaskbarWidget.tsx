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
      {snapshot.limits.length > 0 ? (
        snapshot.limits.map((limit) => (
          <div className="taskbar-quota" key={limit.kind}>
            {/* Every row names its provider: a column header would not fit in the 30-44px
                the taskbar allows, and either provider can be missing entirely. */}
            <span>
              {badge}·{formatWindowBadge(limit.windowDurationMins, limit.label)}
            </span>
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
            {limit.remainingPercent === null ? (
              <i className="unknown" aria-hidden="true" />
            ) : (
              <i aria-label={`${snapshot.displayName} ${limit.label}: ${limit.remainingPercent}% remaining`}>
                <b style={{ width: `${limit.remainingPercent}%`, background: statusColor }} />
              </i>
            )}
          </div>
        ))
      ) : (
        <span className="taskbar-unavailable" style={{ color: statusColor }}>
          {badge} unavailable
        </span>
      )}
    </div>
  );
}

export function TaskbarWidget({ initialWorkspace }: { initialWorkspace: WorkspaceSnapshot }) {
  const { workspace } = useSnapshot(initialWorkspace);
  const providers = workspace.providers;
  const windowCount = Math.max(1, ...providers.map((provider) => provider.limits.length));

  useEffect(() => {
    // The taskbar slice has no room for an error surface; the tray icon and the
    // dashboard remain the recovery surfaces for a failed resize.
    void invoke("set_taskbar_widget_size", {
      providers: Math.max(1, providers.length),
      windows: windowCount,
    }).catch(() => {});
  }, [providers.length, windowCount]);

  return (
    <main
      className={`taskbar-widget-shell${providers.length <= 1 ? " single" : ""}`}
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
