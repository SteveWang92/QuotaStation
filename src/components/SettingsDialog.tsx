import { X } from "lucide-react";
import { useEffect, useRef } from "react";
import { ClaudeFinishedNotifications, ClaudeStatusLine } from "./ClaudeStatusLine";
import { DiagnosticsPanel } from "./DiagnosticsPanel";
import type { DiagnosticsSnapshot, ProviderSnapshot } from "../types";

interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
  /** Whether Claude Code left anything on this machine; its settings are pointless if not. */
  showClaude: boolean;
  diagnostics: DiagnosticsSnapshot;
  providers: ProviderSnapshot[];
  interfaceError: string | null;
}

/**
 * Where the quota sources are configured and where the acquisition paths report. Both are
 * occasional: they are read when something needs setting up or explaining, not while the
 * quota is being watched, so they belong behind one control rather than above the panels
 * the dashboard exists to show.
 *
 * They are also read together — a source is set up and then checked — so the dialog is one
 * page that scrolls rather than two tabs that hide each other.
 */
export function SettingsDialog({
  open,
  onClose,
  showClaude,
  diagnostics,
  providers,
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
          <h2>Settings and diagnostics</h2>
          <button type="button" className="settings-close" onClick={onClose} aria-label="Close settings">
            <X aria-hidden="true" />
          </button>
        </header>
        <div className="settings-body">
          <section aria-label="Quota sources">
            {showClaude ? (
              <>
                <ClaudeStatusLine />
                <ClaudeFinishedNotifications />
              </>
            ) : (
              <p className="settings-empty">
                QuotaStation reads whichever provider clients this machine has. Nothing here
                needs setting up for the ones it found.
              </p>
            )}
          </section>
          <section aria-label="Diagnostics">
            <h3 className="settings-section-heading">Diagnostics</h3>
            <DiagnosticsPanel
              diagnostics={diagnostics}
              providers={providers}
              interfaceError={interfaceError}
            />
          </section>
        </div>
      </div>
    </div>
  );
}
