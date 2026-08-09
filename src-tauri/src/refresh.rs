use std::sync::Arc;

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::{AppState, domain::Freshness, providers::codex};

pub async fn refresh_all(app: &AppHandle, state: &Arc<AppState>) -> crate::domain::ProviderSnapshot {
    let _guard = state.refresh_lock.lock().await;
    let started_at = now();
    {
        let mut snapshot = state.snapshot.write().await;
        snapshot.last_attempt_at = Some(started_at.clone());
    }

    let (live_result, history_result) = tokio::join!(codex::read_live(), codex::read_history());
    apply_live(state, &started_at, live_result).await;
    apply_history(state, &started_at, history_result).await;

    let mut snapshot = state.snapshot.write().await;
    snapshot.freshness = match (&snapshot.live_error, &snapshot.history_error, snapshot.last_success_at.is_some()) {
        (None, None, true) => Freshness::Fresh,
        (_, _, true) => Freshness::Stale,
        _ => Freshness::Unavailable,
    };
    let current = snapshot.clone();
    drop(snapshot);
    let _ = app.emit("snapshot-updated", &current);
    current
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
            let save_error = state.storage.save_history(&history, &completed_at).await.err().map(|e| e.to_string());
            let mut snapshot = state.snapshot.write().await;
            snapshot.today = history.usage;
            snapshot.models = history.models;
            snapshot.api_equivalent_cost_usd = (snapshot.today.total > 0).then_some(history.cost_usd);
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
