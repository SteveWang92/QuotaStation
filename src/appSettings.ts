import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { errorMessage } from "./errors";
import type { AppSettings } from "./types";

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
interface AppSettingsState {
  settings: AppSettings | null;
  error: string | null;
}

const listeners = new Set<(state: AppSettingsState) => void>();

function snapshot(): AppSettingsState {
  return { settings: current, error: loadError };
}

function notify() {
  const state = snapshot();
  for (const listener of listeners) listener(state);
}

function publish(next: AppSettings) {
  current = next;
  loadError = null;
  notify();
}

/** The settings as last read or written, plus any failure from the initial read. */
export function useAppSettings(): AppSettingsState {
  const [state, setState] = useState(snapshot);

  useEffect(() => {
    listeners.add(setState);
    setState(snapshot());
    if (current === null && loading === null) {
      loading = invoke<AppSettings>("get_app_settings")
        .then(publish)
        .catch((cause) => {
          loadError = errorMessage(cause);
          notify();
        })
        .finally(() => {
          loading = null;
        });
    }
    return () => {
      listeners.delete(setState);
    };
  }, []);

  return state;
}

/** Records a change to some of the settings and hands every card the saved result. */
export async function saveAppSettings(patch: Partial<AppSettings>): Promise<void> {
  const save = async () => {
    // Merge only when this write reaches the front of the queue. Two cards can be changed
    // independently, and capturing `current` before the preceding write returns would send
    // an older complete record that quietly undoes it.
    publish(await invoke<AppSettings>("set_app_settings", { settings: { ...current!, ...patch } }));
  };
  const pending = saveQueue.then(save);
  // Keep the queue usable after a rejected write while returning that rejection to the
  // caller whose setting failed.
  saveQueue = pending.catch(() => {});
  return pending;
}
