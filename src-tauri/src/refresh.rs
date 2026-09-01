use std::{sync::Arc, time::Instant};

use anyhow::Result;
use tauri::{AppHandle, Emitter};

use crate::{
    AppState, demo,
    domain::WorkspaceSnapshot,
    providers::{self, ProviderKind},
    sanitize::sanitize_error,
};

const PROVIDER_FALLBACK: &str = "Provider refresh failed";
const STORAGE_FALLBACK: &str = "Local storage write failed";
const RESET_HISTORY_FALLBACK: &str = "Quota reset history unavailable";

/// Refreshes every enabled provider at once. A provider that fails leaves the others
/// untouched, so one broken client never blanks the whole display. A demo instance reads no
/// provider at all — the two readers below decline, and every caller still republishes the
/// seeded snapshot, so a refresh a screenshot session performs by hand changes nothing.
pub async fn refresh_all(app: &AppHandle, state: &Arc<AppState>) -> WorkspaceSnapshot {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    // A client can be installed, or signed in for the first time, while this is running.
    let providers = AppState::local_providers();
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
    if !AppState::local_providers().contains(&provider) {
        return;
    }
    refresh_live_for(state, &[provider]).await;
    publish_snapshot(app, state).await;
}

pub async fn refresh_history(app: &AppHandle, state: &Arc<AppState>) {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    refresh_history_for(state, &AppState::local_providers()).await;
    publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
}

async fn refresh_live_for(state: &Arc<AppState>, providers: &[ProviderKind]) {
    if demo::requested() {
        return;
    }
    let _guard = state.live_refresh_lock.lock().await;
    let started_at = now();
    // A provider whose quota is switched off is not asked for one. This is the single gate:
    // every scheduler, watcher and manual refresh reaches the client through here.
    let tracked = state.quota_providers();
    for &provider in providers.iter().filter(|provider| tracked.contains(provider)) {
        state
            .with_snapshot(provider, |snapshot| {
                snapshot.last_attempt_at = Some(started_at.clone());
            })
            .await;
        let began = Instant::now();
        let live = providers::read_live(provider).await;
        crate::log::write(describe_live(provider, began, &live));
        apply_live(state, provider, &started_at, live).await;
    }
}

async fn refresh_history_for(state: &Arc<AppState>, providers: &[ProviderKind]) {
    if demo::requested() {
        return;
    }
    let _guard = state.history_refresh_lock.lock().await;
    let started_at = now();
    for &provider in providers {
        state
            .with_snapshot(provider, |snapshot| {
                snapshot.last_attempt_at = Some(started_at.clone());
            })
            .await;
        let began = Instant::now();
        let history = providers::read_history(provider).await;
        crate::log::write(describe_history(provider, began, &history));
        apply_history(state, provider, &started_at, history).await;
    }
    // The parse has just replaced this machine's rows. Publishing them and reading in what
    // the other machines published belongs to the same refresh, so the snapshot below
    // counts every machine rather than this one and yesterday's news from the rest.
    let shared_folder = crate::sync::run(state).await;
    *state.shared_folder_diagnostics.write().await = shared_folder;
    if let Err(error) = state.refresh_enabled_providers().await {
        crate::log::write(format!("usage providers could not be refreshed: {error:#}"));
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
    if !AppState::local_providers().contains(&provider) {
        return;
    }
    refresh_history_for(state, &[provider]).await;
    if provider.live_follows_logs() {
        refresh_live_for(state, &[provider]).await;
    }
    publish_snapshot(app, state).await;
    let _ = app.emit("history-updated", ());
}

/// Publishes the snapshot again after something other than a read changed what it holds —
/// switching a provider off is the whole of it, and the columns have to go at once.
pub async fn republish(app: &AppHandle, state: &Arc<AppState>) {
    let _publish_guard = state.refresh_publish_lock.lock().await;
    publish_snapshot(app, state).await;
}

/// What a live read came back with, in one line: the shape of the answer, how long the
/// provider took to give it, and the reason when there was no answer at all.
fn describe_live(
    provider: ProviderKind,
    began: Instant,
    result: &Result<crate::domain::LiveSnapshot>,
) -> String {
    let took = began.elapsed().as_millis();
    match result {
        Ok(live) => format!(
            "{} live read in {took}ms: {} window(s), plan {:?}, {:?} earned reset(s)",
            provider.key(),
            live.limits.len(),
            live.plan_type,
            live.earned_reset_count
        ),
        Err(error) if providers::is_sign_in_required(error) => {
            format!("{} live read in {took}ms: signed out", provider.key())
        }
        Err(error) => format!("{} live read failed in {took}ms: {error:#}", provider.key()),
    }
}

/// The same for a history parse, counted in what it produced rather than in what it read:
/// the file names and their contents are exactly what must not reach this file.
fn describe_history(
    provider: ProviderKind,
    began: Instant,
    result: &Result<(crate::domain::HistorySnapshot, String)>,
) -> String {
    let took = began.elapsed().as_millis();
    match result {
        Ok((history, timezone)) => format!(
            "{} history parsed in {took}ms: {} day(s) in {timezone}",
            provider.key(),
            history.days.len()
        ),
        Err(error) => format!("{} history parse failed in {took}ms: {error:#}", provider.key()),
    }
}

async fn publish_snapshot(app: &AppHandle, state: &Arc<AppState>) -> WorkspaceSnapshot {
    let workspace = state.workspace_snapshot().await;
    // One line for the state every surface is about to draw, which is what makes a window
    // that drew something unexpected answerable from the log rather than from a guess.
    crate::log::write(format!(
        "snapshot published, {}: {}",
        workspace.aggregate.label,
        workspace
            .providers
            .iter()
            .map(|provider| format!(
                "{} {} window(s) {} today{}",
                provider.provider.key(),
                provider.limits.len(),
                provider.today.total,
                match (&provider.live_error, provider.sign_in_required, provider.quota_disabled) {
                    (Some(error), _, _) => format!(" ({error})"),
                    (None, true, _) => " (signed out)".to_string(),
                    (None, false, true) => " (quota off)".to_string(),
                    _ => String::new(),
                }
            ))
            .collect::<Vec<_>>()
            .join("; ")
    ));
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
    let mut signed_out = None;
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
            let (recent_resets, reset_error) =
                match state.storage.load_recent_resets(provider).await {
                    Ok(resets) => (Some(resets), None),
                    Err(error) => {
                        (None, Some(sanitize_error(&error.to_string(), RESET_HISTORY_FALLBACK)))
                    }
                };
            state
                .with_snapshot(provider, |snapshot| {
                    snapshot.plan_type = live.plan_type;
                    snapshot.limits = live.limits;
                    snapshot.earned_reset_count = live.earned_reset_count;
                    snapshot.earned_reset_expires_at = live.earned_reset_expires_at;
                    if let Some(recent_resets) = recent_resets {
                        snapshot.recent_resets = recent_resets;
                    }
                    snapshot.live_error = save_error.or(reset_error);
                    snapshot.sign_in_required = false;
                    if snapshot.live_error.is_none() {
                        snapshot.last_live_success_at = Some(completed_at.clone());
                    }
                })
                .await;
        }
        // The provider answered that this machine is signed out. Nothing is broken, so the
        // snapshot carries the state rather than an error and the surfaces ask for a
        // sign-in instead of reporting a fault. The acquisition path still records the
        // refusal, because the diagnostics panel is where a path that stopped answering
        // has to be visible.
        Err(error) if providers::is_sign_in_required(&error) => {
            state
                .with_snapshot(provider, |snapshot| {
                    snapshot.sign_in_required = true;
                    snapshot.live_error = None;
                })
                .await;
            signed_out = Some(sanitize_error(&error.to_string(), PROVIDER_FALLBACK));
        }
        Err(error) => {
            let message = sanitize_error(&error.to_string(), PROVIDER_FALLBACK);
            state
                .with_snapshot(provider, |snapshot| {
                    snapshot.live_error = Some(message);
                    snapshot.sign_in_required = false;
                })
                .await;
        }
    }
    let error =
        state.read_snapshot(provider, |snapshot| snapshot.live_error.clone()).await.or(signed_out);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Freshness, HistoryDay, HistorySnapshot, LimitKind, LimitWindow, LiveSnapshot, ModelUsage,
        ModelUsageRow, ProviderSnapshot, QuotaLevel, TokenUsage, WindowSource,
    };
    use crate::storage::test_support::{TempDatabase, open_storage};

    const CODEX: ProviderKind = ProviderKind::Codex;

    async fn state() -> (Arc<AppState>, TempDatabase) {
        let (storage, database) = open_storage().await;
        (Arc::new(AppState::for_tests(storage)), database)
    }

    fn window(used_percent: f64) -> LimitWindow {
        LimitWindow {
            kind: LimitKind::Primary,
            label: "5-hour".to_string(),
            used_percent: Some(used_percent),
            window_duration_mins: Some(300),
            resets_at: Some(1_800_000_000),
            source: WindowSource::AppServer,
            observed_at: 1_799_000_000,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
        }
    }

    fn usage(total: u64) -> TokenUsage {
        TokenUsage { input: total, cache_read: 0, output: 0, reasoning: 0, total }
    }

    fn day(date: &str, total: u64, cost_usd: f64) -> HistoryDay {
        HistoryDay {
            date: date.to_string(),
            usage: usage(total),
            models: vec![ModelUsage { model: "gpt-5".to_string(), tokens: total, percent: 100.0 }],
            cost_usd,
            model_rows: vec![ModelUsageRow {
                model: "gpt-5".to_string(),
                input: total,
                cache_read: 0,
                output: 0,
                reasoning: 0,
                total,
                cost_usd,
            }],
        }
    }

    fn today() -> String {
        jiff::Zoned::now().date().to_string()
    }

    async fn snapshot_of(state: &Arc<AppState>) -> ProviderSnapshot {
        state.read_snapshot(CODEX, Clone::clone).await
    }

    /// What the settings page reports for one acquisition path, as its status and error.
    async fn diagnostics_for(state: &Arc<AppState>, path: &str) -> (String, Option<String>) {
        let entry = state
            .storage
            .load_acquisition_diagnostics(CODEX)
            .await
            .expect("load diagnostics")
            .into_iter()
            .find(|entry| entry.acquisition_path == path)
            .expect("the refresh recorded this path");
        (entry.status, entry.error)
    }

    #[tokio::test]
    async fn a_failed_quota_read_keeps_the_last_windows_on_screen() {
        let (state, _database) = state().await;
        state
            .with_snapshot(CODEX, |snapshot| {
                snapshot.limits = vec![window(42.0)];
                snapshot.plan_type = Some("Plus".to_string());
                snapshot.last_live_success_at = Some("2026-08-30T01:00:00Z".to_string());
            })
            .await;

        apply_live(&state, CODEX, "2026-08-31T01:00:00Z", Err(anyhow::anyhow!("connection reset")))
            .await;

        let snapshot = snapshot_of(&state).await;
        assert_eq!(snapshot.limits.len(), 1, "the reading before the failure is still displayed");
        assert_eq!(snapshot.limits[0].used_percent, Some(42.0));
        assert_eq!(snapshot.plan_type.as_deref(), Some("Plus"));
        assert_eq!(snapshot.live_error.as_deref(), Some("connection reset"));
        assert_eq!(
            snapshot.last_live_success_at.as_deref(),
            Some("2026-08-30T01:00:00Z"),
            "a failure does not count as a success",
        );
    }

    #[tokio::test]
    async fn a_failed_read_never_reports_a_local_path() {
        let (state, _database) = state().await;

        apply_live(
            &state,
            CODEX,
            "2026-08-31T01:00:00Z",
            Err(anyhow::anyhow!("cannot read C:\\Users\\example\\.codex\\auth.json")),
        )
        .await;

        let message = snapshot_of(&state).await.live_error.expect("the failure is reported");
        assert_eq!(message, "cannot read <path>");
    }

    #[tokio::test]
    async fn a_successful_quota_read_replaces_the_windows_and_clears_the_failure() {
        let (state, _database) = state().await;
        state
            .with_snapshot(CODEX, |snapshot| {
                snapshot.limits = vec![window(42.0)];
                snapshot.live_error = Some("connection reset".to_string());
            })
            .await;

        apply_live(
            &state,
            CODEX,
            "2026-08-31T01:00:00Z",
            Ok(LiveSnapshot {
                plan_type: Some("Pro".to_string()),
                limits: vec![window(61.0)],
                earned_reset_count: Some(2),
                earned_reset_expires_at: Some(1_800_100_000),
            }),
        )
        .await;

        let snapshot = snapshot_of(&state).await;
        assert_eq!(snapshot.limits[0].used_percent, Some(61.0));
        assert_eq!(snapshot.plan_type.as_deref(), Some("Pro"));
        assert_eq!(snapshot.earned_reset_count, Some(2));
        assert_eq!(snapshot.live_error, None);
        assert!(snapshot.last_live_success_at.is_some());
    }

    #[tokio::test]
    async fn every_quota_read_is_recorded_against_its_acquisition_path() {
        let (state, _database) = state().await;

        apply_live(&state, CODEX, "2026-08-31T01:00:00Z", Err(anyhow::anyhow!("connection reset")))
            .await;
        let (status, error) = diagnostics_for(&state, &CODEX.live_path()).await;
        assert_eq!(error.as_deref(), Some("connection reset"));
        assert_ne!(status, "ok", "a failed read is not reported as a healthy one");

        apply_live(
            &state,
            CODEX,
            "2026-08-31T01:05:00Z",
            Ok(LiveSnapshot {
                plan_type: None,
                limits: vec![window(10.0)],
                earned_reset_count: None,
                earned_reset_expires_at: None,
            }),
        )
        .await;
        let (_, error) = diagnostics_for(&state, &CODEX.live_path()).await;
        assert_eq!(error, None, "the next successful read clears the recorded failure");
    }

    #[tokio::test]
    async fn todays_row_becomes_the_totals_the_surfaces_show() {
        let (state, _database) = state().await;

        apply_history(
            &state,
            CODEX,
            "2026-08-31T01:00:00Z",
            Ok((
                HistorySnapshot {
                    days: vec![day("2026-08-29", 500, 0.05), day(&today(), 1_200, 0.42)],
                    hours: Vec::new(),
                },
                "Australia/Sydney".to_string(),
            )),
        )
        .await;

        let snapshot = snapshot_of(&state).await;
        assert_eq!(snapshot.today.total, 1_200, "yesterday's row is not what today shows");
        assert_eq!(snapshot.api_equivalent_cost_usd, Some(0.42));
        assert_eq!(snapshot.models.len(), 1);
        assert_eq!(snapshot.history_error, None);
        assert!(snapshot.last_history_success_at.is_some());
    }

    #[tokio::test]
    async fn a_day_with_no_usage_yet_shows_no_cost() {
        let (state, _database) = state().await;

        apply_history(
            &state,
            CODEX,
            "2026-08-31T01:00:00Z",
            Ok((
                HistorySnapshot { days: vec![day(&today(), 0, 0.0)], hours: Vec::new() },
                "Australia/Sydney".to_string(),
            )),
        )
        .await;

        assert_eq!(snapshot_of(&state).await.api_equivalent_cost_usd, None);
    }

    #[tokio::test]
    async fn a_new_day_starts_the_totals_again() {
        let (state, _database) = state().await;
        state
            .with_snapshot(CODEX, |snapshot| {
                snapshot.today = usage(1_200);
                snapshot.api_equivalent_cost_usd = Some(0.42);
                snapshot.models =
                    vec![ModelUsage { model: "gpt-5".to_string(), tokens: 1_200, percent: 100.0 }];
            })
            .await;

        apply_history(
            &state,
            CODEX,
            "2026-08-31T01:00:00Z",
            Ok((
                HistorySnapshot { days: vec![day("2026-08-29", 500, 0.05)], hours: Vec::new() },
                "Australia/Sydney".to_string(),
            )),
        )
        .await;

        let snapshot = snapshot_of(&state).await;
        assert_eq!(snapshot.today.total, 0, "yesterday's total does not carry into today");
        assert_eq!(snapshot.api_equivalent_cost_usd, None);
        assert!(snapshot.models.is_empty());
    }

    #[tokio::test]
    async fn a_failed_history_read_keeps_the_totals_already_parsed() {
        let (state, _database) = state().await;
        state
            .with_snapshot(CODEX, |snapshot| {
                snapshot.today = usage(1_200);
                snapshot.last_history_success_at = Some("2026-08-30T01:00:00Z".to_string());
            })
            .await;

        apply_history(
            &state,
            CODEX,
            "2026-08-31T01:00:00Z",
            Err(anyhow::anyhow!("session log unreadable")),
        )
        .await;

        let snapshot = snapshot_of(&state).await;
        assert_eq!(snapshot.today.total, 1_200);
        assert_eq!(snapshot.history_error.as_deref(), Some("session log unreadable"));
        assert_eq!(
            snapshot.last_history_success_at.as_deref(),
            Some("2026-08-30T01:00:00Z"),
            "a failure does not count as a success",
        );
    }
}
