import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { errorMessage } from "./errors";
import type { AppSettings } from "./types";

const FIRST_RETRY_MS = 250;
const MAX_RETRY_MS = 5_000;

/**
 * One copy of the settings for the whole window.
 *
 * The dialog shows several cards that each own a different field of the same record, and
 * `set_app_settings` takes the record whole: a card writing back the copy it read at mount
 * would quietly undo whatever another card changed since. They share this instead.
 */
let current: AppSettings | null = null;
let loadError: string | null = null;
let loading: Promise<unknown> | null = null;
let saveQueue: Promise<void> = Promise.resolve();
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let retryDelay = FIRST_RETRY_MS;
export interface AppSettingsState {
  settings: AppSettings | null;
  error: string | null;
  reload: () => Promise<void>;
}

type StoredSettingsState = Omit<AppSettingsState, "reload">;
const listeners = new Set<(state: StoredSettingsState) => void>();

function state(): StoredSettingsState {
  return { settings: current, error: loadError };
}

function notify() {
  const next = state();
  for (const listener of listeners) listener(next);
}

function publish(next: AppSettings) {
  current = next;
  loadError = null;
  retryDelay = FIRST_RETRY_MS;
  notify();
}

/** Reads settings again after a visible startup or IPC failure. */
export async function reloadAppSettings(): Promise<void> {
  if (loading !== null) {
    await loading;
    return;
  }
  loading = invoke<AppSettings>("get_app_settings")
    .then(publish)
    .catch((cause) => {
      loadError = errorMessage(cause);
      notify();
      // The window exists before the core has state to answer with, so the first read of a
      // freshly launched window can be rejected for a reason that passes on its own. Until
      // one lands there is no record to change, and every card that writes one is refused.
      if (retryTimer === undefined) {
        retryTimer = setTimeout(() => {
          retryTimer = undefined;
          void reloadAppSettings();
        }, retryDelay);
        retryDelay = Math.min(retryDelay * 2, MAX_RETRY_MS);
      }
    })
    .finally(() => {
      loading = null;
    });
  await loading;
}

/** The settings as last read or written, with a visible failure and retry path. */
export function useAppSettings(): AppSettingsState {
  const [stored, setStored] = useState(state);

  useEffect(() => {
    listeners.add(setStored);
    setStored(state());
    if (current === null && loading === null) {
      void reloadAppSettings();
    }
    return () => {
      listeners.delete(setStored);
    };
  }, []);

  return { ...stored, reload: reloadAppSettings };
}

/** Records a change to some of the settings and hands every card the saved result. */
export async function saveAppSettings(
  patch: Partial<AppSettings> | ((current: AppSettings) => Partial<AppSettings>),
): Promise<void> {
  const save = async () => {
    // The record is replaced wholesale by the core, and every field it carries has a serde
    // default, so a patch sent before the first read landed would reset the device identity
    // and the shared folder rather than change one setting.
    if (current === null) throw new Error("Settings have not loaded");
    // A patch built from a list has to be built here rather than at the call site: two
    // clicks before the first result renders would otherwise both start from the same
    // stale list and the second would undo the first.
    const fields = typeof patch === "function" ? patch(current) : patch;
    // Merge only when this write reaches the front of the queue. Two cards can be changed
    // independently, and capturing `current` before the preceding write returns would send
    // an older complete record that quietly undoes it.
    publish(await invoke<AppSettings>("set_app_settings", { settings: { ...current, ...fields } }));
  };
  const pending = saveQueue.then(save);
  // Keep the queue usable after a rejected write while returning that rejection to the
  // caller whose setting failed.
  saveQueue = pending.catch(() => {});
  return pending;
}
