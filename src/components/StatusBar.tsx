import { SlidersHorizontal } from "lucide-react";

interface StatusBarProps {
  attention: boolean;
  onOpenSettings: () => void;
}

/**
 * Only the control. Every provider panel above already carries its own status in its own
 * header, so anything this bar said about quota was the same sentence a second time — and
 * a bar that reports nothing has no reason to be tinted either.
 */
export function StatusBar({ attention, onOpenSettings }: StatusBarProps) {
  return (
    <footer className="status-bar">
      <div className="status-summary">
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
