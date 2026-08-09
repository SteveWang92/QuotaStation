use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use ccusage_adapter_codex::codex_usage_paths;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::AppHandle;
use tokio::{sync::mpsc, time::Instant};

use crate::{AppState, refresh};

enum WatcherMessage {
    HistoryChanged,
    Failed,
}

pub fn start(app: AppHandle, state: Arc<AppState>) -> Result<()> {
    let locations = codex_usage_paths()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    anyhow::ensure!(!locations.is_empty(), "no Codex session locations are available");

    let (sender, receiver) = mpsc::unbounded_channel();
    let event_sender = sender.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| match result {
        Ok(event) if is_history_event(&event) => {
            let _ = event_sender.send(WatcherMessage::HistoryChanged);
        }
        Ok(_) => {}
        Err(_) => {
            let _ = event_sender.send(WatcherMessage::Failed);
        }
    })
    .context("create Codex session watcher")?;

    let mut watched_location_count = 0;
    for location in &locations {
        if watcher.watch(location, RecursiveMode::Recursive).is_ok() {
            watched_location_count += 1;
        }
    }
    anyhow::ensure!(watched_location_count > 0, "Codex session locations could not be watched");

    tauri::async_runtime::block_on(async {
        let mut diagnostics = state.watcher_diagnostics.write().await;
        diagnostics.status = "active".to_string();
        diagnostics.watched_location_count = watched_location_count;
        diagnostics.error = None;
    });

    std::thread::Builder::new()
        .name("codex-session-watcher".to_string())
        .spawn(move || {
            let _watcher: RecommendedWatcher = watcher;
            std::thread::park();
        })
        .context("start Codex session watcher thread")?;

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
            WatcherMessage::HistoryChanged => {
                mark_event(&state).await;
                let mut deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => break,
                        next = receiver.recv() => match next {
                            Some(WatcherMessage::HistoryChanged) => {
                                mark_event(&state).await;
                                deadline = Instant::now() + Duration::from_secs(2);
                            }
                            Some(WatcherMessage::Failed) => mark_failed(&state).await,
                            None => return,
                        }
                    }
                }
                refresh::refresh_history(&app, &state).await;
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
    diagnostics.error = Some("The operating system reported a session watcher error.".to_string());
}
