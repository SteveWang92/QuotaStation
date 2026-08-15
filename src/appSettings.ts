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
        .catch(() => {
          // Every card that needs the settings hides itself until they arrive, so a failed
          // read costs the cards and nothing else.
        })
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
  if (current === null) return;
  publish(await invoke<AppSettings>("set_app_settings", { settings: { ...current, ...patch } }));
}
