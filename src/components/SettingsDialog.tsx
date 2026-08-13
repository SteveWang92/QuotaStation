import { X } from "lucide-react";
import { useEffect, useRef } from "react";
import { ClaudeStatusLine } from "./ClaudeStatusLine";
import { DiagnosticsPanel } from "./DiagnosticsPanel";
import type { DiagnosticsSnapshot } from "../types";

export type SettingsTab = "settings" | "diagnostics";

interface SettingsDialogProps {
  open: boolean;
  tab: SettingsTab;
  onSelectTab: (tab: SettingsTab) => void;
  onClose: () => void;
  /** Whether Claude Code left anything on this machine; its settings are pointless if not. */
  showClaude: boolean;
  diagnostics: DiagnosticsSnapshot;
  interfaceError: string | null;
}

/**
 * Where the quota sources are configured and where the acquisition paths report. Both are
 * occasional: they are read when something needs setting up or explaining, not while the
 * quota is being watched, so they belong behind one control rather than above the panels
 * the dashboard exists to show.
 */
export function SettingsDialog({
  open,
  tab,
  onSelectTab,
  onClose,
  showClaude,
  diagnostics,
  interfaceError,
}: SettingsDialogProps) {
  const panel = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    // The dialog is opened from the tray as well as from the status bar, so focus moves
    // here rather than staying wherever the window happened to leave it.
    panel.current?.focus();
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="settings-overlay" onMouseDown={onClose}>
      <div
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="QuotaStation settings"
        tabIndex={-1}
        ref={panel}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="settings-header">
          <div className="settings-tabs" role="tablist">
            <button
              type="button"
              role="tab"
              aria-selected={tab === "settings"}
              className={tab === "settings" ? "active" : ""}
              onClick={() => onSelectTab("settings")}
            >
              Settings
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={tab === "diagnostics"}
              className={tab === "diagnostics" ? "active" : ""}
              onClick={() => onSelectTab("diagnostics")}
            >
              Diagnostics
            </button>
          </div>
          <button type="button" className="settings-close" onClick={onClose} aria-label="Close settings">
            <X aria-hidden="true" />
          </button>
        </header>
        <div className="settings-body">
          {tab === "settings" ? (
            showClaude ? (
              <ClaudeStatusLine />
            ) : (
              <p className="settings-empty">
                QuotaStation reads whichever provider clients this machine has. Nothing here
                needs setting up for the ones it found.
              </p>
            )
          ) : (
            <DiagnosticsPanel diagnostics={diagnostics} interfaceError={interfaceError} />
          )}
        </div>
      </div>
    </div>
  );
}
