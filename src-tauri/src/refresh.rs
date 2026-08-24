use std::sync::Arc;

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::{
    AppState,
    domain::WorkspaceSnapshot,
    providers::{self, ProviderKind},
    sanitize::sanitize_error,
};

const PROVIDER_FALLBACK: &str = "Provider refresh failed";
const STORAGE_FALLBACK: &str = "Local storage write failed";

/// Refreshes every enabled provider at once. A provider that fails leaves the others
/// untouched, so one broken client never blanks the whole display.
pub async fn refresh_all(app: &AppHandle, state: &Arc<AppState>) -> WorkspaceSnapshot {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    // A client can be installed, or signed in for the first time, while this is running.
    let providers = state.detect_providers();
    tokio::join!(refresh_live_for(state, &providers), refresh_history_for(state, &providers));
    let workspace = publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
    workspace
}

/// Refreshes one provider's live quota, for the schedulers that run each provider on its
/// own interval.
pub async fn refresh_live_for_provider(
    app: &AppHandle,
    state: &Arc<AppState>,
    provider: ProviderKind,
) {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    if !state.enabled_providers().contains(&provider) {
        return;
    }
    refresh_live_for(state, &[provider]).await;
    publish_snapshot(app, state).await;
}

pub async fn refresh_history(app: &AppHandle, state: &Arc<AppState>) {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    refresh_history_for(state, &state.enabled_providers()).await;
    publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
}

async fn refresh_live_for(state: &Arc<AppState>, providers: &[ProviderKind]) {
    let _guard = state.live_refresh_lock.lock().await;
    let started_at = now();
    for &provider in providers {
        state
            .with_snapshot(provider, |snapshot| {
                snapshot.last_attempt_at = Some(started_at.clone());
            })
            .await;
        let live = providers::read_live(provider).await;
        apply_live(state, provider, &started_at, live).await;
    }
}

async fn refresh_history_for(state: &Arc<AppState>, providers: &[ProviderKind]) {
    let _guard = state.history_refresh_lock.lock().await;
    let started_at = now();
    for &provider in providers {
        state
            .with_snapshot(provider, |snapshot| {
                snapshot.last_attempt_at = Some(started_at.clone());
            })
            .await;
        apply_history(state, provider, &started_at, providers::read_history(provider).await).await;
    }
}

/// A session-file burst may change both history and live quota. Keep the two reads behind
/// one publication so the renderer never sees the brief half-refreshed state between them.
pub async fn refresh_changed_provider(
    app: &AppHandle,
    state: &Arc<AppState>,
    provider: ProviderKind,
) {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    if !state.enabled_providers().contains(&provider) {
        return;
    }
    refresh_history_for(state, &[provider]).await;
    if provider.live_follows_logs() {
        refresh_live_for(state, &[provider]).await;
    }
    publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
}

async fn publish_snapshot(app: &AppHandle, state: &Arc<AppState>) -> WorkspaceSnapshot {
    let workspace = state.workspace_snapshot().await;
    let _ = app.emit("snapshot-updated", &workspace);
    // The event reaches this application's own windows and nothing else. The status-line
    // bridge is a separate process with no way to receive it, so the same snapshot is also
    // left on disk for it to read.
    crate::summary::publish(&workspace);
    // Every refresh passes through here, whichever scheduler or watcher asked for it, so it
    // is the one place that sees every change a notification could be about.
    crate::alerts::review(app, &workspace);
    workspace
}

async fn apply_live(
    state: &Arc<AppState>,
    provider: ProviderKind,
    started_at: &str,
    result: Result<crate::domain::LiveSnapshot>,
) {
    let completed_at = now();
    match result {
        Ok(live) => {
            let mut save_error = state
                .storage
                .save_live(provider, &live, &completed_at)
                .await
                .err()
                .map(storage_error);
            // Reading the restarts back after the save keeps one owner of the detection,
            // so a restart recognised by this very save is already part of the snapshot.
            let recent_resets = if save_error.is_none() {
                match state.storage.load_recent_resets(provider).await {
                    Ok(resets) => Some(resets),
                    Err(error) => {
                        save_error = Some(storage_error(error));
                        None
                    }
                }
            } else {
                None
            };
            state
                .with_snapshot(provider, |snapshot| {
                    snapshot.plan_type = live.plan_type;
                    snapshot.limits = live.limits;
                    snapshot.earned_reset_count = live.earned_reset_count;
                    if let Some(recent_resets) = recent_resets {
                        snapshot.recent_resets = recent_resets;
                    }
                    snapshot.live_error = save_error;
                    if snapshot.live_error.is_none() {
                        snapshot.last_live_success_at = Some(completed_at.clone());
                    }
                })
                .await;
        }
        Err(error) => {
            let message = sanitize_error(&error.to_string(), PROVIDER_FALLBACK);
            state.with_snapshot(provider, |snapshot| snapshot.live_error = Some(message)).await;
        }
    }
    let error = state.read_snapshot(provider, |snapshot| snapshot.live_error.clone()).await;
    if let Err(error) = state
        .storage
        .record_refresh(
            provider,
            &provider.live_path(),
            started_at,
            &completed_at,
            error.as_deref(),
        )
        .await
    {
        crate::log::write(format!("live refresh record failed: {error:#}"));
    }
}

async fn apply_history(
    state: &Arc<AppState>,
    provider: ProviderKind,
    started_at: &str,
    result: Result<(crate::domain::HistorySnapshot, String)>,
) {
    let completed_at = now();
    match result {
        Ok((history, aggregation_timezone)) => {
            let today_date = jiff::Zoned::now().date().to_string();
            let today = history.days.iter().find(|day| day.date == today_date).cloned();
            let save_error = state
                .storage
                .save_history(provider, &history, &aggregation_timezone, &completed_at)
                .await
                .err()
                .map(storage_error);
            state
                .with_snapshot(provider, |snapshot| {
                    if let Some(today) = today {
                        snapshot.today = today.usage;
                        snapshot.models = today.models;
                        snapshot.api_equivalent_cost_usd =
                            (snapshot.today.total > 0).then_some(today.cost_usd);
                    } else {
                        snapshot.today = Default::default();
                        snapshot.models.clear();
                        snapshot.api_equivalent_cost_usd = None;
                    }
                    snapshot.history_error = save_error;
                    if snapshot.history_error.is_none() {
                        snapshot.last_history_success_at = Some(completed_at.clone());
                    }
                })
                .await;
        }
        Err(error) => {
            let message = sanitize_error(&error.to_string(), PROVIDER_FALLBACK);
            state.with_snapshot(provider, |snapshot| snapshot.history_error = Some(message)).await;
        }
    }
    let error = state.read_snapshot(provider, |snapshot| snapshot.history_error.clone()).await;
    if let Err(error) = state
        .storage
        .record_refresh(
            provider,
            &provider.history_path(),
            started_at,
            &completed_at,
            error.as_deref(),
        )
        .await
    {
        crate::log::write(format!("history refresh record failed: {error:#}"));
    }
}

fn storage_error(error: anyhow::Error) -> String {
    sanitize_error(&error.to_string(), STORAGE_FALLBACK)
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}
