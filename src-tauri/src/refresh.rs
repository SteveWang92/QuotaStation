use std::sync::Arc;

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::{AppState, domain::Freshness, providers::codex};

pub async fn refresh_all(app: &AppHandle, state: &Arc<AppState>) -> crate::domain::ProviderSnapshot {
    tokio::join!(refresh_live(app, state), refresh_history(app, state));
    state.snapshot.read().await.clone()
}

pub async fn refresh_live(app: &AppHandle, state: &Arc<AppState>) {
    let _guard = state.live_refresh_lock.lock().await;
    let started_at = now();
    {
        let mut snapshot = state.snapshot.write().await;
        snapshot.last_attempt_at = Some(started_at.clone());
    }

    apply_live(state, &started_at, codex::read_live().await).await;
    publish_snapshot(app, state).await;
}

pub async fn refresh_history(app: &AppHandle, state: &Arc<AppState>) {
    let _guard = state.history_refresh_lock.lock().await;
    let started_at = now();
    {
        let mut snapshot = state.snapshot.write().await;
        snapshot.last_attempt_at = Some(started_at.clone());
    }
    apply_history(state, &started_at, codex::read_history().await).await;
    publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
}

async fn publish_snapshot(app: &AppHandle, state: &Arc<AppState>) {
    let mut snapshot = state.snapshot.write().await;
    snapshot.freshness = match (&snapshot.live_error, &snapshot.history_error, snapshot.last_success_at.is_some()) {
        (None, None, true) => Freshness::Fresh,
        (_, _, true) => Freshness::Stale,
        _ => Freshness::Unavailable,
    };
    let current = snapshot.clone();
    drop(snapshot);
    let _ = app.emit("snapshot-updated", &current);
}

async fn apply_live(state: &Arc<AppState>, started_at: &str, result: Result<crate::domain::LiveSnapshot>) {
    let completed_at = now();
    match result {
        Ok(live) => {
            let save_error = state.storage.save_live(&live, &completed_at).await.err().map(|e| e.to_string());
            let mut snapshot = state.snapshot.write().await;
            snapshot.plan_type = live.plan_type;
            snapshot.limits = live.limits;
            snapshot.earned_reset_count = live.earned_reset_count;
            snapshot.live_error = save_error;
            if snapshot.live_error.is_none() { snapshot.last_success_at = Some(completed_at.clone()); }
        }
        Err(error) => state.snapshot.write().await.live_error = Some(sanitize_error(&error.to_string())),
    }
    let error = state.snapshot.read().await.live_error.clone();
    let _ = state.storage.record_refresh("codex_live", started_at, &completed_at, error.as_deref()).await;
}

async fn apply_history(state: &Arc<AppState>, started_at: &str, result: Result<crate::domain::HistorySnapshot>) {
    let completed_at = now();
    match result {
        Ok(history) => {
            let today_date = jiff::Zoned::now().date().to_string();
            let today = history.days.iter().find(|day| day.date == today_date).cloned();
            let save_error = state.storage.save_history(&history, &completed_at).await.err().map(|e| e.to_string());
            let mut snapshot = state.snapshot.write().await;
            if let Some(today) = today {
                snapshot.today = today.usage;
                snapshot.models = today.models;
                snapshot.api_equivalent_cost_usd = (snapshot.today.total > 0).then_some(today.cost_usd);
            } else {
                snapshot.today = Default::default();
                snapshot.models.clear();
                snapshot.api_equivalent_cost_usd = None;
            }
            snapshot.history_error = save_error;
            if snapshot.history_error.is_none() { snapshot.last_success_at = Some(completed_at.clone()); }
        }
        Err(error) => state.snapshot.write().await.history_error = Some(sanitize_error(&error.to_string())),
    }
    let error = state.snapshot.read().await.history_error.clone();
    let _ = state.storage.record_refresh("codex_history", started_at, &completed_at, error.as_deref()).await;
}

fn sanitize_error(error: &str) -> String {
    let line = error.lines().next().unwrap_or("Provider refresh failed");
    if line.len() > 220 { format!("{}…", &line[..220]) } else { line.to_string() }
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}
