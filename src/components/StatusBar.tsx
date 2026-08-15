import { AlertTriangle, SlidersHorizontal } from "lucide-react";
import type { CompactStatus } from "../types";

interface StatusBarProps {
  /** The workspace status, so the bar reads the same whether one provider is shown or two. */
  status: CompactStatus;
  /** Marks the bar when something an acquisition path reported needs looking at. */
  attention: boolean;
  onOpenSettings: () => void;
}

/**
 * The provider panels already carry each provider's own status, so a healthy workspace has
 * nothing left for the bar to add and it keeps only the control. Anything else is worth a
 * sentence, because that is the case the panels above cannot fully explain on their own.
 */
export function StatusBar({ status, attention, onOpenSettings }: StatusBarProps) {
  const healthy = status.level === "healthy";
  return (
    <footer className={`status-bar ${status.level}`} style={{ "--status-color": status.color } as React.CSSProperties}>
      <div className="status-summary">
        {healthy ? null : (
          <>
            <AlertTriangle aria-hidden="true" />
            <strong>{status.label}</strong>
            <span className="status-message">{status.message}</span>
          </>
        )}
        <button
          type="button"
          className={`settings-toggle${attention ? " attention" : ""}`}
          onClick={onOpenSettings}
        >
          <SlidersHorizontal aria-hidden="true" />
          Settings and diagnostics
        </button>
      </div>
    </footer>
  );
}
