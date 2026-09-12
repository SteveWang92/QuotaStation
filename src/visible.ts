import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Whether this window is on screen right now.
 *
 * Every window here is hidden rather than closed — the dashboard once it is dismissed, the
 * quick panel between clicks, the status widget while it is switched off — and a hidden one
 * goes on reconciling and re-reading for a surface nobody can see.
 *
 * The question goes to the window rather than to `document.visibilityState`, because a
 * webview inside a hidden native window is not reliably told that it is hidden.
 */
export async function onScreen(): Promise<boolean> {
  // A window that can no longer answer is one Explorer or Tauri has already destroyed, and
  // that is not on screen either.
  return getCurrentWindow()
    .isVisible()
    .catch(() => false);
}
