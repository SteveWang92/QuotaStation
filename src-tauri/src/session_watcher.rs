use std::{collections::BTreeSet, sync::Arc, time::Duration};

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
    Failed,
}

pub fn start(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let mut watched_location_count = 0;
    let mut watchers = Vec::new();

    for provider in state.enabled_providers() {
        let locations = provider
            .usage_paths()
            .unwrap_or_default()
            .into_iter()
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        if locations.is_empty() {
            continue;
        }

        let event_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                Ok(event) if is_history_event(&event) => {
                    let _ = event_sender.send(WatcherMessage::HistoryChanged(provider));
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = event_sender.send(WatcherMessage::Failed);
                }
            })
            .context("create session watcher")?;

        for location in &locations {
            if watcher.watch(location, RecursiveMode::Recursive).is_ok() {
                watched_location_count += 1;
            }
        }
        watchers.push(watcher);
    }

    anyhow::ensure!(
        watched_location_count > 0,
        "no provider session locations could be watched"
    );

    tauri::async_runtime::block_on(async {
        let mut diagnostics = state.watcher_diagnostics.write().await;
        diagnostics.status = "active".to_string();
        diagnostics.watched_location_count = watched_location_count;
        diagnostics.error = None;
    });

    std::thread::Builder::new()
        .name("session-watcher".to_string())
        .spawn(move || {
            let _watchers: Vec<RecommendedWatcher> = watchers;
            std::thread::park();
        })
        .context("start session watcher thread")?;

    tauri::async_runtime::spawn(run_event_loop(app, state, receiver));
    Ok(())
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
}
