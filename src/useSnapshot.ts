import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { errorMessage } from "./errors";
import type { WorkspaceSnapshot } from "./types";

const POLL_INTERVAL_MS = 30_000;
const FIRST_RETRY_MS = 250;
const MAX_RETRY_MS = 5_000;

/**
 * Shared workspace subscription for every window: one read on mount, push updates
 * from the core, and a periodic reconciliation poll.
 *
 * Tauri creates the configured windows before the core finishes its setup, so the
 * first read of a freshly launched window can be rejected while the core is still
 * opening its database. Those reads are retried with a backoff instead of leaving
 * the window on its placeholder snapshot.
 */
export function useSnapshot(
  initialSnapshot: WorkspaceSnapshot,
  onSnapshot?: (snapshot: WorkspaceSnapshot) => void,
) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [error, setError] = useState<string | null>(null);
  // A workspace with no providers means something different before and after the first
  // answer: still starting, or no provider client on this machine.
  const [loaded, setLoaded] = useState(false);
  const onSnapshotRef = useRef(onSnapshot);

  useEffect(() => {
    onSnapshotRef.current = onSnapshot;
  }, [onSnapshot]);

  useEffect(() => {
    let disposed = false;
    let stopListening = () => {};
    let retryTimer: number | undefined;
    let retryDelay = FIRST_RETRY_MS;

    const apply = (next: WorkspaceSnapshot) => {
      setSnapshot(next);
      setLoaded(true);
      setError(null);
      retryDelay = FIRST_RETRY_MS;
      onSnapshotRef.current?.(next);
    };

    const load = async () => {
      try {
        const next = await invoke<WorkspaceSnapshot>("get_snapshot");
        if (!disposed) apply(next);
      } catch (cause) {
        if (disposed) return;
        setError(errorMessage(cause));
        if (retryTimer !== undefined) return;
        retryTimer = window.setTimeout(() => {
          retryTimer = undefined;
          void load();
        }, retryDelay);
        retryDelay = Math.min(retryDelay * 2, MAX_RETRY_MS);
      }
    };

    void load();
    void listen<WorkspaceSnapshot>("snapshot-updated", ({ payload }) => {
      if (!disposed) apply(payload);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stopListening = unlisten;
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      });

    const poll = window.setInterval(() => void load(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      stopListening();
      window.clearInterval(poll);
      window.clearTimeout(retryTimer);
    };
  }, []);

  return { workspace: snapshot, error, loaded };
}
