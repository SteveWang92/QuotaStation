import { useEffect, useRef, useState } from "react";
import type { DiagnosticsSnapshot, ProviderSnapshot } from "../types";
import { AboutPanel } from "./AboutPanel";
import { ClaudeFinishedNotifications, ClaudeStatusLine } from "./ClaudeStatusLine";
import { DiagnosticsPanel } from "./DiagnosticsPanel";
import { GeneralSettings } from "./GeneralSettings";
import { QuotaNotifications } from "./QuotaNotifications";
import { ResetHistoryPanel } from "./ResetHistoryPanel";

interface SettingsPageProps {
  /** Whether Claude Code left anything on this machine; its settings are pointless if not. */
  showClaude: boolean;
  diagnostics: DiagnosticsSnapshot;
  providers: ProviderSnapshot[];
  interfaceError: string | null;
}

const SECTIONS = [
  { id: "application", label: "Application" },
  { id: "notifications", label: "Notifications" },
  { id: "sources", label: "Quota sources" },
  { id: "resets", label: "Reset history" },
  { id: "diagnostics", label: "Diagnostics" },
  { id: "about", label: "About" },
] as const;

/**
 * Every setting the application has, where the acquisition paths report, and the full
 * record of quota-window restarts.
 *
 * It is one page that scrolls rather than tabs that hide each other, because it is read
 * together: a source is set up and then checked, and a restart is explained by the
 * diagnostics beside it. The nav down the left is only a way to jump — nothing it lists is
 * hidden from a search of the page or from scrolling past it. The tray menu keeps only what
 * has to work with no window open, so no preference has two homes.
 */
export function SettingsPage({
  showClaude,
  diagnostics,
  providers,
  interfaceError,
}: SettingsPageProps) {
  const [active, setActive] = useState<string>(SECTIONS[0].id);
  const body = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const root = body.current;
    if (root === null) return;
    // Whichever section heading is nearest the top of the scroller is the one being read.
    // A section shorter than the viewport never fills it, so the intersection ratio says
    // nothing useful; its position does.
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((entry) => entry.isIntersecting);
        if (visible.length === 0) return;
        setActive(
          visible.reduce((nearest, entry) =>
            entry.boundingClientRect.top < nearest.boundingClientRect.top ? entry : nearest,
          ).target.id,
        );
      },
      { root, rootMargin: "0px 0px -70% 0px" },
    );
    for (const section of SECTIONS) {
      const element = root.querySelector(`#${section.id}`);
      if (element !== null) observer.observe(element);
    }
    return () => observer.disconnect();
  }, []);

  return (
    <div className="settings-page">
      <nav className="settings-nav" aria-label="Settings sections">
        {SECTIONS.map((section) => (
          <button
            type="button"
            key={section.id}
            className={section.id === active ? "active" : ""}
            aria-current={section.id === active}
            onClick={() =>
              body.current
                ?.querySelector(`#${section.id}`)
                ?.scrollIntoView({ behavior: "smooth", block: "start" })
            }
          >
            {section.label}
          </button>
        ))}
      </nav>
      <div className="settings-body" ref={body}>
        <section id="application" aria-label="Application">
          <h3 className="settings-section-heading">Application</h3>
          <GeneralSettings />
        </section>
        <section id="notifications" aria-label="Notifications">
          <h3 className="settings-section-heading">Notifications</h3>
          <QuotaNotifications />
          {showClaude ? <ClaudeFinishedNotifications /> : null}
        </section>
        <section id="sources" aria-label="Quota sources">
          <h3 className="settings-section-heading">Quota sources</h3>
          {showClaude ? (
            <ClaudeStatusLine />
          ) : (
            <p className="settings-empty">
              QuotaStation reads whichever provider clients this machine has. Nothing here needs
              setting up for the ones it found.
            </p>
          )}
        </section>
        <section id="resets" aria-label="Reset history">
          <h3 className="settings-section-heading">Reset history</h3>
          <ResetHistoryPanel />
        </section>
        <section id="diagnostics" aria-label="Diagnostics">
          <h3 className="settings-section-heading">Diagnostics</h3>
          <DiagnosticsPanel
            diagnostics={diagnostics}
            providers={providers}
            interfaceError={interfaceError}
          />
        </section>
        <section id="about" aria-label="About">
          <h3 className="settings-section-heading">About</h3>
          <AboutPanel appVersion={diagnostics.appVersion} buildCommit={diagnostics.buildCommit} />
        </section>
      </div>
    </div>
  );
}
