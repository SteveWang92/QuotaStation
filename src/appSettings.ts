import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { AppSettings } from "./types";

/**
 * One copy of the settings for the whole window.
 *
 * The dialog shows several cards that each own a different field of the same record, and
 * `set_app_settings` takes the record whole: a card writing back the copy it read at mount
 * would quietly undo whatever another card changed since. They share this instead.
 */
let current: AppSettings | null = null;
let loading: Promise<unknown> | null = null;
let saveQueue: Promise<void> = Promise.resolve();
const listeners = new Set<(settings: AppSettings | null) => void>();

function publish(next: AppSettings) {
  current = next;
  for (const listener of listeners) listener(next);
}

/** The settings as last read or written, or `null` until the first read lands. */
export function useAppSettings(): AppSettings | null {
  const [settings, setSettings] = useState(current);

  useEffect(() => {
    listeners.add(setSettings);
    setSettings(current);
    if (current === null && loading === null) {
      loading = invoke<AppSettings>("get_app_settings")
        .then(publish)
        .finally(() => {
          loading = null;
        });
    }
    return () => {
      listeners.delete(setSettings);
    };
  }, []);

  return settings;
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
