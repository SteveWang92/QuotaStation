use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::AppHandle;
use tokio::{sync::mpsc, time::Instant};

use crate::{
    AppState,
    domain::WatcherDiagnostics,
    providers::ProviderKind,
    refresh,
};

enum WatcherMessage {
    /// Which provider's session files changed, so only that history is reparsed.
    HistoryChanged(ProviderKind),
    LocationsChanged(usize),
    Failed,
}

type WatchKey = (ProviderKind, PathBuf);

pub fn start(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let mut watchers = BTreeMap::new();
    let failed = Arc::new(StdMutex::new(BTreeSet::new()));
    let (_, complete) = reconcile_watchers(&state, &sender, &failed, &mut watchers);
    report_reconcile(&sender, complete, watchers.len());

    let manager_state = state.clone();
    std::thread::Builder::new()
        .name("session-watcher".to_string())
        .spawn(move || {
            loop {
                std::thread::park_timeout(Duration::from_secs(60));
                let (changed, complete) =
                    reconcile_watchers(&manager_state, &sender, &failed, &mut watchers);
                if changed || !complete {
                    report_reconcile(&sender, complete, watchers.len());
                }
            }
        })
        .context("start session watcher thread")?;

    tauri::async_runtime::spawn(run_event_loop(app, state, receiver));
    Ok(())
}

fn reconcile_watchers(
    state: &AppState,
    sender: &mpsc::UnboundedSender<WatcherMessage>,
    failed: &Arc<StdMutex<BTreeSet<WatchKey>>>,
    watchers: &mut BTreeMap<WatchKey, RecommendedWatcher>,
) -> (bool, bool) {
    let mut desired = BTreeSet::new();
    let mut discovery_complete = true;
    for provider in state.detect_providers() {
        let Ok(locations) = provider.usage_paths() else {
            discovery_complete = false;
            let _ = sender.send(WatcherMessage::Failed);
            continue;
        };
        for location in locations.into_iter().filter(|path| path.is_dir()) {
            let location = std::fs::canonicalize(&location).unwrap_or(location);
            desired.insert((provider, location));
        }
    }

    let previous = watchers.keys().cloned().collect::<BTreeSet<_>>();
    let failed_keys = {
        let mut keys = failed.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *keys)
    };
    for key in &failed_keys {
        watchers.remove(key);
    }
    watchers.retain(|key, _| desired.contains(key));
    for key in desired.iter().cloned() {
        if watchers.contains_key(&key) {
            continue;
        }
        let provider = key.0;
        let location = key.1.clone();
        let event_sender = sender.clone();
        let event_failed = failed.clone();
        let failure_key = key.clone();
        let Ok(mut watcher) = notify::recommended_watcher(
            move |result: notify::Result<Event>| match result {
                Ok(event) if is_history_event(&event) => {
                    let _ = event_sender.send(WatcherMessage::HistoryChanged(provider));
                }
                Ok(_) => {}
                Err(_) => {
                    event_failed
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(failure_key.clone());
                    let _ = event_sender.send(WatcherMessage::Failed);
                }
            },
        ) else {
            let _ = sender.send(WatcherMessage::Failed);
            continue;
        };
        if watcher.watch(&location, RecursiveMode::Recursive).is_err() {
            let _ = sender.send(WatcherMessage::Failed);
            continue;
        }
        watchers.insert(key, watcher);
    }
    let current = watchers.keys().cloned().collect::<BTreeSet<_>>();
    let complete = discovery_complete && current == desired;
    (previous != current || !failed_keys.is_empty(), complete)
}

fn report_reconcile(
    sender: &mpsc::UnboundedSender<WatcherMessage>,
    complete: bool,
    watched_location_count: usize,
) {
    let message = if complete {
        WatcherMessage::LocationsChanged(watched_location_count)
    } else {
        WatcherMessage::Failed
    };
    let _ = sender.send(message);
}

fn is_history_event(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && event.paths.iter().any(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        })
}

async fn run_event_loop(
    app: AppHandle,
    state: Arc<AppState>,
    mut receiver: mpsc::UnboundedReceiver<WatcherMessage>,
) {
    while let Some(message) = receiver.recv().await {
        match message {
            WatcherMessage::Failed => mark_failed(&state).await,
            WatcherMessage::LocationsChanged(count) => mark_locations(&state, count).await,
            WatcherMessage::HistoryChanged(provider) => {
                // A burst of writes across providers is one settling period, and each
                // provider that took part is reparsed once when it ends.
                let mut pending = BTreeSet::from([provider]);
                mark_event(&state).await;
                let mut deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => break,
                        next = receiver.recv() => match next {
                            Some(WatcherMessage::HistoryChanged(provider)) => {
                                pending.insert(provider);
                                mark_event(&state).await;
                                deadline = Instant::now() + Duration::from_secs(2);
                            }
                            Some(WatcherMessage::Failed) => mark_failed(&state).await,
                            Some(WatcherMessage::LocationsChanged(count)) => mark_locations(&state, count).await,
                            None => return,
                        }
                    }
                }
                while let Some(provider) = pending.pop_first() {
                    refresh::refresh_history_for_provider(&app, &state, provider).await;
                    // A provider whose quota window is derived from those same files has
                    // a new window to report as soon as they change.
                    if provider.live_follows_logs() {
                        refresh::refresh_live_for_provider(&app, &state, provider).await;
                    }
                    // Parsing can take long enough for more writes to arrive. Fold those
                    // events into this batch instead of making every queued event open a
                    // fresh two-second debounce window.
                    while let Ok(message) = receiver.try_recv() {
                        match message {
                            WatcherMessage::HistoryChanged(provider) => {
                                pending.insert(provider);
                                mark_event(&state).await;
                            }
                            WatcherMessage::Failed => mark_failed(&state).await,
                            WatcherMessage::LocationsChanged(count) => mark_locations(&state, count).await,
                        }
                    }
                }
            }
        }
    }
}

async fn mark_event(state: &Arc<AppState>) {
    let mut diagnostics = state.watcher_diagnostics.write().await;
    diagnostics.last_event_at = Some(jiff::Timestamp::now().to_string());
}

async fn mark_failed(state: &Arc<AppState>) {
    let mut diagnostics = state.watcher_diagnostics.write().await;
    mark_failed_diagnostics(&mut diagnostics);
}

async fn mark_locations(state: &Arc<AppState>, count: usize) {
    let mut diagnostics = state.watcher_diagnostics.write().await;
    update_location_diagnostics(&mut diagnostics, count);
}

fn update_location_diagnostics(diagnostics: &mut WatcherDiagnostics, count: usize) {
    diagnostics.watched_location_count = count;
    if count == 0 {
        diagnostics.status = "unavailable".to_string();
        diagnostics.error = Some(
            "No provider session location exists yet; periodic discovery remains active.".to_string(),
        );
    } else {
        diagnostics.status = "active".to_string();
        diagnostics.error = None;
    }
}

fn mark_failed_diagnostics(diagnostics: &mut WatcherDiagnostics) {
    diagnostics.status = "degraded".to_string();
    diagnostics.error = Some("The operating system reported a session watcher error.".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_failure_is_not_reported_as_active() {
        let mut diagnostics = WatcherDiagnostics {
            status: "active".to_string(),
            watched_location_count: 1,
            last_event_at: None,
            error: None,
        };
        mark_failed_diagnostics(&mut diagnostics);
        assert_eq!(diagnostics.status, "degraded");
        assert!(diagnostics.error.is_some());
    }

    #[test]
    fn watcher_diagnostics_recover_when_a_location_appears() {
        let mut diagnostics = WatcherDiagnostics::default();
        update_location_diagnostics(&mut diagnostics, 0);
        assert_eq!(diagnostics.status, "unavailable");
        assert!(diagnostics.error.is_some());

        update_location_diagnostics(&mut diagnostics, 2);
        assert_eq!(diagnostics.status, "active");
        assert_eq!(diagnostics.watched_location_count, 2);
        assert!(diagnostics.error.is_none());
    }

    #[test]
    fn incomplete_reconcile_stays_degraded_until_recovery() {
        let (sender, mut receiver) = mpsc::unbounded_channel();

        report_reconcile(&sender, false, 1);
        assert!(matches!(receiver.try_recv(), Ok(WatcherMessage::Failed)));

        report_reconcile(&sender, true, 1);
        assert!(matches!(
            receiver.try_recv(),
            Ok(WatcherMessage::LocationsChanged(1))
        ));
    }
}
