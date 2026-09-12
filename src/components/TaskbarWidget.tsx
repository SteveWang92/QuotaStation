import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { statusColor } from "../theme";
import type { ProviderSnapshot, WorkspaceSnapshot } from "../types";
import { useSnapshot } from "../useSnapshot";
import { QuotaGlanceRow } from "./QuotaGlanceRow";

function ProviderColumn({ snapshot }: { snapshot: ProviderSnapshot }) {
  const providerColor = statusColor(snapshot.compactStatus);
  return (
    <div className="taskbar-provider">
      {/* One name for the whole column rather than one per row: the name is the widest
          thing in a slot the taskbar may only give 30px of. */}
      <span className="taskbar-name" style={{ color: providerColor }}>
        {snapshot.shortName}
      </span>
      <div className="taskbar-windows">
        {/* A signed-out provider keeps the last reading it managed, and the core deliberately
            leaves it there. Drawing it here would show a percentage and a countdown that
            stopped being true hours ago, so this slot says what the other two surfaces say. */}
        {snapshot.signInRequired ? (
          <span className="taskbar-unavailable" style={{ color: providerColor }}>
            signed out
          </span>
        ) : snapshot.limits.length > 0 ? (
          snapshot.limits.map((limit) => (
            <QuotaGlanceRow
              key={limit.kind}
              limit={limit}
              label={`${snapshot.displayName} ${limit.label}`}
              fallbackColor={providerColor}
            />
          ))
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
  // The widget shows quota and nothing else, so a provider whose quota is switched off
  // takes no slot in it rather than reserving one that can only say "unavailable".
  const providers = workspace.providers.filter((provider) => !provider.quotaDisabled);

  useEffect(() => {
    // Rust owns the slot width and reserves the existing two-provider capacity. Passing only
    // the normalized provider count lets future providers grow by a complete slot without
    // making the renderer responsible for native taskbar geometry.
    // Resizing a native window can fail, and an unhandled rejection in the widget is
    // invisible: the size it already has is the honest fallback.
    void invoke("set_taskbar_widget_size", { providerCount: providers.length }).catch(() => {});
  }, [providers.length]);

  return (
    <main
      className="taskbar-widget-shell"
      style={{ "--taskbar-status-color": statusColor(workspace.aggregate) } as React.CSSProperties}
    >
      {providers.length > 0 ? (
        providers.map((snapshot) => <ProviderColumn key={snapshot.provider} snapshot={snapshot} />)
      ) : (
        <span className="taskbar-unavailable">
          {workspace.providers.length > 0 ? "Quota tracking off" : "No provider detected"}
        </span>
      )}
    </main>
  );
}
