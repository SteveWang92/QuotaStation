import { invoke } from "@tauri-apps/api/core";

/**
 * What happened in a window, written to the log the core keeps.
 *
 * Everything a window does begins here and reaches the core only as whichever command it
 * ends in, so a window that drew nothing — or a script that threw before it drew anything —
 * leaves no trace at all without this. The core redacts and truncates what it is given, and
 * a failure to record is swallowed: logging must never become a second fault to report.
 */
export function logActivity(detail: string): void {
  void invoke("log_activity", { detail }).catch(() => {});
}

/**
 * Sends this window's uncaught failures to the same log.
 *
 * A release build has no console anyone can open, so an exception during the first render
 * is invisible from outside: the window is simply empty. This is what makes that state
 * answerable afterwards rather than only while someone is watching it happen.
 */
export function reportRendererFailures(label: string): void {
  window.addEventListener("error", (event) => {
    logActivity(
      `${label} window failed: ${event.message} (${event.lineno}:${event.colno}) ${
        event.error instanceof Error ? event.error.stack : ""
      }`,
    );
  });
  window.addEventListener("unhandledrejection", (event) => {
    const reason: unknown = event.reason;
    logActivity(
      `${label} window rejected: ${reason instanceof Error ? (reason.stack ?? reason.message) : String(reason)}`,
    );
  });
}
