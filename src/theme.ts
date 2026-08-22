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
  void invoke<ThemeSnapshot>("get_theme")
    .then((theme) => applyTheme(theme, isTaskbarWidget))
    .catch(() => {
      // The document already carries the dark default from the stylesheet, so a failed
      // read costs the choice and never the readability.
    });
  void listen<ThemeSnapshot>("theme-changed", ({ payload }) => {
    applyTheme(payload, isTaskbarWidget);
  });
}

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
