import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { ProviderSnapshot } from "../types";

function shortLabel(durationMins: number | null, fallback: string) {
  if (durationMins === null) return fallback.slice(0, 2).toUpperCase();
  if (durationMins % 1_440 === 0) return `${durationMins / 1_440}D`;
  if (durationMins % 60 === 0) return `${durationMins / 60}H`;
  return `${durationMins}M`;
}

function compactReset(resetsAt: number | null) {
  if (resetsAt === null) return "—";
  const hours = Math.max(0, Math.ceil((resetsAt * 1_000 - Date.now()) / 3_600_000));
  const days = Math.floor(hours / 24);
  return days > 0 ? `${days}d ${hours % 24}h` : `${hours}h`;
}

export function TaskbarWidget({ initialSnapshot }: { initialSnapshot: ProviderSnapshot }) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const loadSnapshot = useCallback(async () => setSnapshot(await invoke<ProviderSnapshot>("get_snapshot")), []);

  useEffect(() => {
    let disposed = false;
    let stop = () => {};
    void loadSnapshot();
    void listen<ProviderSnapshot>("snapshot-updated", ({ payload }) => setSnapshot(payload)).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    const timer = window.setInterval(() => void loadSnapshot(), 30_000);
    return () => { disposed = true; stop(); window.clearInterval(timer); };
  }, [loadSnapshot]);

  useEffect(() => {
    void invoke("set_taskbar_widget_columns", { columns: Math.max(1, snapshot.limits.length) });
  }, [snapshot.limits.length]);

  return (
    <main className={`taskbar-widget-shell${snapshot.limits.length <= 1 ? " single" : ""}`} style={{ "--taskbar-status-color": snapshot.compactStatus.color } as React.CSSProperties}>
      {snapshot.limits.length > 0 ? snapshot.limits.map((limit) => (
        <div className="taskbar-quota" key={limit.kind}>
          <span>{shortLabel(limit.windowDurationMins, limit.label)}</span>
          <strong>{limit.remainingPercent === null ? "—" : `${Math.round(limit.remainingPercent)}%`}</strong>
          <time dateTime={limit.resetsAt === null ? undefined : new Date(limit.resetsAt * 1_000).toISOString()}>
            {compactReset(limit.resetsAt)}
          </time>
          <i aria-label={`${limit.remainingPercent ?? 0}% remaining`}>
            <b style={{ width: `${limit.remainingPercent ?? 0}%` }} />
          </i>
        </div>
      )) : <span className="taskbar-unavailable">Codex unavailable</span>}
    </main>
  );
}
