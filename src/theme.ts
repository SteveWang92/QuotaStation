import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CompactStatus, LimitWindow, ThemeSnapshot } from "./types";

/**
 * Which palette this window draws in.
 *
 * The core resolves it and every window is told, rather than each one deciding for itself:
 * `prefers-color-scheme` inside a WebView2 answers with the theme the application just set
 * on the window, so a renderer asked to work it out would only ever hear its own echo. The
 * core reads what Windows is actually set to and publishes both answers at once — one for
 * the windows that follow the user's choice, one for the taskbar widget, which follows the
 * taskbar it is drawn on instead.
 */
export function applyTheme(theme: ThemeSnapshot, isTaskbarWidget: boolean): void {
  document.documentElement.dataset.theme = isTaskbarWidget ? theme.taskbar : theme.app;
}

/**
 * Keeps this window's palette current for as long as it is open. Called once, outside
 * React: the attribute belongs to the document rather than to any component, and stamping
 * it from an effect would leave the first paint in the wrong theme.
 */
export function watchTheme(isTaskbarWidget: boolean): void {
  // Tauri creates the windows before the core finishes its setup, so this first read can be
  // rejected while there is still no state to answer it — which a window that asked once
  // would wear as the wrong palette until something else changed the theme.
  const read = (delay: number, attemptsLeft: number) => {
    void invoke<ThemeSnapshot>("get_theme")
      .then((theme) => applyTheme(theme, isTaskbarWidget))
      .catch(() => {
        // Bounded, because the race this covers is over in seconds. A refusal that outlives
        // the budget is not the one described above, and the theme-changed event below still
        // corrects the palette the moment anything changes it.
        if (attemptsLeft > 0) {
          setTimeout(() => read(Math.min(delay * 2, MAX_THEME_RETRY_MS), attemptsLeft - 1), delay);
        }
      });
  };
  read(FIRST_THEME_RETRY_MS, MAX_THEME_RETRIES);
  void listen<ThemeSnapshot>("theme-changed", ({ payload }) => {
    applyTheme(payload, isTaskbarWidget);
  });
}

const FIRST_THEME_RETRY_MS = 250;
const MAX_THEME_RETRY_MS = 5_000;
const MAX_THEME_RETRIES = 12;

/**
 * What a level is drawn in, as the token rather than the colour.
 *
 * The core says how loud a reading is and stops there, because the same 95% is one red on a
 * near-black dashboard and another on a white one. Resolving that here is what lets a single
 * snapshot be drawn correctly in either theme.
 */
export function quotaColor(limit: LimitWindow): string {
  return `var(--status-${limit.statusLevel})`;
}

/** The same mapping for a provider's overall status, which has two levels more. */
export function statusColor(status: CompactStatus): string {
  switch (status.level) {
    case "healthy":
      return "var(--status-healthy)";
    case "warning":
    case "stale":
      return "var(--status-warning)";
    default:
      return "var(--status-critical)";
  }
}
