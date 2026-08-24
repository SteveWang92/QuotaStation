import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { formatCompactCountdown, formatWindowBadge } from "../format";
import { quotaColor, statusColor } from "../theme";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";

function ProviderColumn({ snapshot }: { snapshot: ProviderSnapshot }) {
  const providerColor = statusColor(snapshot.compactStatus);
  return (
    <div className="taskbar-provider">
      {/* One name for the whole column rather than one per row: a provider showing both of
          its windows was repeating its own name, which is the widest thing in a slot the
          taskbar may only give 30px of. */}
      <span className="taskbar-name" style={{ color: providerColor }}>
        {snapshot.shortName}
      </span>
      <div className="taskbar-windows">
        {snapshot.limits.length > 0 ? (
          snapshot.limits.map((limit) => {
            // Every window draws the same three fixed-width cells — badge, bar, reading — so
            // the bars of two windows, and of two providers, all start and end on the same
            // pixel. A proportional bar made each row a different length instead.
            const percent = limit.usedPercent === null ? null : `${Math.round(limit.usedPercent)}%`;
            const countdown =
              limit.resetsAt === null ? null : formatCompactCountdown(limit.resetsAt);
            return (
              <div className="taskbar-quota" key={limit.kind}>
                <span className="taskbar-window-badge">
                  {formatWindowBadge(limit.windowDurationMins, limit.label)}
                </span>
                {/* The bar fills with what has been consumed, exactly as the dashboard's meter
                    does. Filling it with what is left would leave the same quota drawn one way
                    in the taskbar and the other way in the window above it. */}
                {limit.usedPercent === null ? (
                  <i className="unknown" aria-hidden="true" />
                ) : (
                  <i aria-hidden="true">
                    <b
                      style={{
                        width: `${limit.usedPercent}%`,
                        background: quotaColor(limit),
                      }}
                    />
                  </i>
                )}
                {/* Usage and reset share one centred cell so a window still waiting for its
                    first reading shows a single dash on the same axis as the window below it,
                    rather than two dashes pushed against the right edge. */}
                <span className="taskbar-reading">
                  {percent === null && countdown === null ? (
                    <em
                      role="img"
                      aria-label={`${snapshot.displayName} ${limit.label}: no reading yet`}
                      style={{ color: providerColor }}
                    >
                      —
                    </em>
                  ) : (
                    <>
                      {percent !== null && <em style={{ color: quotaColor(limit) }}>{percent}</em>}
                      {percent !== null && countdown !== null && (
                        <span className="taskbar-dot" aria-hidden="true">
                          ·
                        </span>
                      )}
                      {countdown !== null && limit.resetsAt !== null && (
                        <time dateTime={new Date(limit.resetsAt * 1_000).toISOString()}>
                          {countdown}
                        </time>
                      )}
                    </>
                  )}
                </span>
              </div>
            );
          })
        ) : (
          <span className="taskbar-unavailable" style={{ color: providerColor }}>
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
    // Rust owns the slot width and reserves the existing two-provider capacity. Passing only
    // the normalized provider count lets future providers grow by a complete slot without
    // making the renderer responsible for native taskbar geometry.
    void invoke("set_taskbar_widget_size", { providerCount: providers.length });
  }, [providers.length]);

  return (
    <main
      className="taskbar-widget-shell"
      style={{ "--taskbar-status-color": statusColor(workspace.aggregate) } as React.CSSProperties}
    >
      {providers.length > 0 ? (
        providers.map((snapshot) => <ProviderColumn key={snapshot.provider} snapshot={snapshot} />)
      ) : (
        <span className="taskbar-unavailable">No provider detected</span>
      )}
    </main>
  );
}
