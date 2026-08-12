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
    let providers = state.enabled_providers();
    tokio::join!(
        refresh_live_for(app, state, &providers),
        refresh_history_for(app, state, &providers)
    );
    state.workspace_snapshot().await
}

pub async fn refresh_live(app: &AppHandle, state: &Arc<AppState>) {
    refresh_live_for(app, state, &state.enabled_providers()).await;
}

pub async fn refresh_history(app: &AppHandle, state: &Arc<AppState>) {
    refresh_history_for(app, state, &state.enabled_providers()).await;
}

async fn refresh_live_for(app: &AppHandle, state: &Arc<AppState>, providers: &[ProviderKind]) {
    let _guard = state.live_refresh_lock.lock().await;
    let started_at = now();
    for &provider in providers {
        state
            .with_snapshot(provider, |snapshot| {
                snapshot.last_attempt_at = Some(started_at.clone());
            })
            .await;
        apply_live(state, provider, &started_at, providers::read_live(provider).await).await;
    }
    publish_snapshot(app, state).await;
}

async fn refresh_history_for(app: &AppHandle, state: &Arc<AppState>, providers: &[ProviderKind]) {
    let _guard = state.history_refresh_lock.lock().await;
    let started_at = now();
    for &provider in providers {
        state
            .with_snapshot(provider, |snapshot| {
                snapshot.last_attempt_at = Some(started_at.clone());
            })
            .await;
        apply_history(
            state,
            provider,
            &started_at,
            providers::read_history(provider).await,
        )
        .await;
    }
    publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
}

/// Refreshes one provider's history, for the watcher that knows which files changed.
pub async fn refresh_history_for_provider(
    app: &AppHandle,
    state: &Arc<AppState>,
    provider: ProviderKind,
) {
    if !state.enabled_providers().contains(&provider) {
        return;
    }
    refresh_history_for(app, state, &[provider]).await;
}

async fn publish_snapshot(app: &AppHandle, state: &Arc<AppState>) {
    let workspace = state.workspace_snapshot().await;
    let _ = app.emit("snapshot-updated", &workspace);
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
            let save_error = state
                .storage
                .save_live(provider, &live, &completed_at)
                .await
                .err()
                .map(storage_error);
            // Reading the restarts back after the save keeps one owner of the detection,
            // so a restart recognised by this very save is already part of the snapshot.
            let recent_resets = state
                .storage
                .load_recent_resets(provider)
                .await
                .unwrap_or_default();
            state
                .with_snapshot(provider, |snapshot| {
                    snapshot.plan_type = live.plan_type;
                    snapshot.limits = live.limits;
                    snapshot.earned_reset_count = live.earned_reset_count;
                    snapshot.recent_resets = recent_resets;
                    snapshot.live_error = save_error;
                    if snapshot.live_error.is_none() {
                        snapshot.last_success_at = Some(completed_at.clone());
                    }
                })
                .await;
        }
        Err(error) => {
            let message = sanitize_error(&error.to_string(), PROVIDER_FALLBACK);
            state
                .with_snapshot(provider, |snapshot| snapshot.live_error = Some(message))
                .await;
        }
    }
    let error = state
        .read_snapshot(provider, |snapshot| snapshot.live_error.clone())
        .await;
    let _ = state
        .storage
        .record_refresh(
            provider,
            &provider.live_path(),
            started_at,
            &completed_at,
            error.as_deref(),
        )
        .await;
}

async fn apply_history(
    state: &Arc<AppState>,
    provider: ProviderKind,
    started_at: &str,
    result: Result<crate::domain::HistorySnapshot>,
) {
    let completed_at = now();
    match result {
        Ok(history) => {
            let today_date = jiff::Zoned::now().date().to_string();
            let today = history.days.iter().find(|day| day.date == today_date).cloned();
            let save_error = state
                .storage
                .save_history(provider, &history, &completed_at)
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
                        snapshot.last_success_at = Some(completed_at.clone());
                    }
                })
                .await;
        }
        Err(error) => {
            let message = sanitize_error(&error.to_string(), PROVIDER_FALLBACK);
            state
                .with_snapshot(provider, |snapshot| snapshot.history_error = Some(message))
                .await;
        }
    }
    let error = state
        .read_snapshot(provider, |snapshot| snapshot.history_error.clone())
        .await;
    let _ = state
        .storage
        .record_refresh(
            provider,
            &provider.history_path(),
            started_at,
            &completed_at,
            error.as_deref(),
        )
        .await;
}

fn storage_error(error: anyhow::Error) -> String {
    sanitize_error(&error.to_string(), STORAGE_FALLBACK)
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}
