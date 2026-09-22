//! Everything the renderer can ask for.
//!
//! One narrow command per question, each returning normalized data: no file path, no raw
//! session content and no credential crosses this boundary.

use crate::settings::AppSettings;

use std::{path::PathBuf, sync::Arc};

use crate::domain::{
    DeviceDiagnostics, DiagnosticsSnapshot, QuotaHistorySnapshot, SessionCostSnapshot,
    UsageHoursSnapshot, UsageRangeSnapshot, UsageWindowSnapshot, WorkspaceSnapshot,
};
use crate::providers::{ProviderKind, claude::notifications, claude::statusline};
use tauri::{Emitter, Manager, State};

use crate::{
    AppState, apply_theme, build_kind, demo, diagnostic_export, domain, log, refresh, sanitize,
    schedule_taskbar_widget_placement, set_taskbar_widget_visible, settings, storage, taskbar,
    theme,
};

#[tauri::command]
pub(crate) async fn get_snapshot(
    state: State<'_, Arc<AppState>>,
) -> Result<WorkspaceSnapshot, String> {
    // Not logged: every window re-reads this on a timer, and a line per poll would bury
    // the log in the one event that carries no information.
    Ok(state.workspace_snapshot().await)
}

/// One line for a stored-data query, so a dashboard that drew the wrong thing can be
/// explained from the log rather than from a reproduction.
fn log_query<T, E: std::fmt::Display>(
    request: &str,
    result: &Result<T, E>,
    summarize: impl FnOnce(&T) -> String,
) {
    match result {
        Ok(value) => log::write(format!("query {request}: {}", summarize(value))),
        Err(error) => log::write(format!("query {request} failed: {error}")),
    }
}

/// What a query was asked for, in the vocabulary the commands take it in.
fn query_scope(provider: Option<ProviderKind>, device: Option<&str>) -> String {
    format!(
        "{} on {}",
        provider.map_or("all providers", ProviderKind::key),
        device.map_or("this device", |_| "one device"),
    )
}

/// What the renderer did, in the renderer's own words.
///
/// Everything a window does starts there and reaches the core only as whichever command it
/// ends in, so a window that drew nothing, or a script that threw before it drew anything,
/// leaves no trace at all without this. It is the one way in, it writes to the same file as
/// every other line, and what it is handed is redacted and truncated the same way.
#[tauri::command]
pub(crate) fn log_activity(detail: String) {
    log::write(format!("ui: {detail}"));
}

#[tauri::command]
pub(crate) async fn get_usage_range(
    // No provider is the combined history: the dashboard's "All" tab reads every
    // provider in one query rather than adding up separate answers in the renderer.
    provider: Option<ProviderKind>,
    device: Option<String>,
    start_date: String,
    end_date: String,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageRangeSnapshot, String> {
    let result =
        state.storage.load_usage_range(provider, device.as_deref(), &start_date, &end_date).await;
    log_query(
        &format!(
            "daily usage {start_date}..{end_date} for {}",
            query_scope(provider, device.as_deref())
        ),
        &result,
        |range| format!("{} day(s), {} model(s)", range.days.len(), range.models.len()),
    );
    result.map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_usage_start_date(
    provider: Option<ProviderKind>,
    device: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<String>, String> {
    let result = state.storage.load_usage_start_date(provider, device.as_deref()).await;
    log_query(
        &format!("earliest usage for {}", query_scope(provider, device.as_deref())),
        &result,
        |start| format!("{start:?}"),
    );
    result.map_err(|error| error.to_string())
}

/// The same range hour by hour, for the short ranges the dashboard draws that way.
#[tauri::command]
pub(crate) async fn get_usage_hours(
    provider: Option<ProviderKind>,
    device: Option<String>,
    start_date: String,
    end_date: String,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageHoursSnapshot, String> {
    let result =
        state.storage.load_usage_hours(provider, device.as_deref(), &start_date, &end_date).await;
    log_query(
        &format!(
            "hourly usage {start_date}..{end_date} for {}",
            query_scope(provider, device.as_deref())
        ),
        &result,
        |hours| format!("{} hour(s)", hours.hours.len()),
    );
    result.map_err(|error| error.to_string())
}

/// A rolling window of hours — the last twenty-four of them — with its totals summed
/// from exactly those hours rather than from the two partial days they fall in.
#[tauri::command]
pub(crate) async fn get_usage_window(
    provider: Option<ProviderKind>,
    device: Option<String>,
    start_hour: String,
    end_hour: String,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageWindowSnapshot, String> {
    let result =
        state.storage.load_usage_window(provider, device.as_deref(), &start_hour, &end_hour).await;
    log_query(
        &format!(
            "usage window {start_hour}..{end_hour} for {}",
            query_scope(provider, device.as_deref())
        ),
        &result,
        |window| format!("{} hour(s)", window.hours.hours.len()),
    );
    result.map_err(|error| error.to_string())
}

/// Every restart QuotaStation has recorded, per provider. The dashboard annotates the
/// window running now; this is the list the settings page shows in full.
#[tauri::command]
pub(crate) async fn get_reset_history(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ProviderResetHistory>, String> {
    let mut history = Vec::new();
    for provider in state.enabled_providers() {
        let result = state.storage.load_reset_history(provider).await;
        log_query(&format!("reset history for {}", provider.key()), &result, |resets| {
            format!("{} restart(s)", resets.len())
        });
        history.push(ProviderResetHistory {
            provider,
            display_name: provider.display_name().to_string(),
            resets: result.map_err(|error| error.to_string())?,
        });
    }
    Ok(history)
}

/// One provider's whole restart history, named so the settings page can head the list.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderResetHistory {
    provider: ProviderKind,
    display_name: String,
    resets: Vec<domain::LimitResetEvent>,
}

#[tauri::command]
pub(crate) async fn get_quota_history(
    provider: ProviderKind,
    start_date: String,
    end_date: String,
    state: State<'_, Arc<AppState>>,
) -> Result<QuotaHistorySnapshot, String> {
    let result = state.storage.load_quota_history(provider, &start_date, &end_date).await;
    log_query(
        &format!("quota history {start_date}..{end_date} for {}", provider.key()),
        &result,
        |history| {
            format!("{} window(s), {} restart(s)", history.windows.len(), history.resets.len())
        },
    );
    result.map_err(|error| error.to_string())
}

/// Every session comparison recorded inside a range, for one provider or all of them.
///
/// Only Claude Code writes a cost of its own, so another provider answers with an empty
/// range rather than an error: nothing is wrong, there is simply nothing to compare.
#[tauri::command]
pub(crate) async fn get_session_costs(
    provider: Option<ProviderKind>,
    start_date: String,
    end_date: String,
    state: State<'_, Arc<AppState>>,
) -> Result<SessionCostSnapshot, String> {
    let result = state.storage.session_costs(provider, &start_date, &end_date).await;
    log_query(
        &format!(
            "session costs {start_date}..{end_date} for {}",
            provider.map_or("every provider", |kind| kind.key())
        ),
        &result,
        |snapshot| format!("{} session(s)", snapshot.sessions.len()),
    );
    result.map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn refresh_now(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<WorkspaceSnapshot, String> {
    log::write("refresh requested by hand");
    Ok(refresh::refresh_all(&app, state.inner()).await)
}

/// The displays the status can be shown on. Read live rather than stored: a monitor is
/// attached and detached while the application runs.
#[tauri::command]
pub(crate) fn get_taskbar_displays() -> Vec<taskbar::TaskbarDisplay> {
    taskbar::taskbar_displays()
}

#[tauri::command]
pub(crate) fn set_taskbar_widget_size(
    app: tauri::AppHandle,
    provider_count: u32,
) -> Result<(), String> {
    // A hidden widget still runs its renderer; resizing it must not bring it back.
    if !app.state::<Arc<AppState>>().settings().taskbar_widget_enabled {
        return Ok(());
    }
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(error) = taskbar::set_widget_size(&handle, provider_count) {
            log::write(format!("taskbar status resize: {error}"));
        }
    })
    .map_err(|error| error.to_string())
}

/// A provider whose quota this machine could read, for the switch that decides whether it
/// does. Only a provider with a client here can be asked for a quota at all.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderChoice {
    provider: ProviderKind,
    display_name: String,
}

#[tauri::command]
pub(crate) fn get_provider_choices() -> Vec<ProviderChoice> {
    ProviderKind::ALL
        .into_iter()
        .filter(|provider| demo::requested() || provider.is_installed())
        .map(|provider| ProviderChoice {
            provider,
            display_name: provider.display_name().to_string(),
        })
        .collect()
}

/// The settings as the renderer reads them: the record itself, and the zones it resolves to,
/// which the record cannot carry because they are not choices.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsView {
    #[serde(flatten)]
    settings: AppSettings,
    /// The zone every time is shown in: the chosen one, or the Windows zone.
    resolved_time_zone: String,
    /// The Windows zone, for the entry that follows it.
    system_time_zone: String,
}

impl SettingsView {
    fn of(settings: AppSettings) -> Self {
        Self {
            settings,
            resolved_time_zone: crate::clock::zone_name(),
            system_time_zone: crate::clock::system_zone_name(),
        }
    }
}

#[tauri::command]
pub(crate) fn get_app_settings(state: State<'_, Arc<AppState>>) -> SettingsView {
    SettingsView::of(state.settings())
}

/// Which preferences a save actually moved, named rather than valued wherever the value is
/// this machine's own — a device name or a folder is recorded as changed and no further.
fn settings_changes(previous: &AppSettings, next: &AppSettings) -> Vec<String> {
    let mut changes = Vec::new();
    if previous.theme != next.theme {
        changes.push(format!("theme {:?}", next.theme));
    }
    if previous.taskbar_widget_enabled != next.taskbar_widget_enabled {
        changes.push(format!("taskbar status {}", on_off(next.taskbar_widget_enabled)));
    }
    if previous.taskbar_widget_display != next.taskbar_widget_display {
        changes.push("taskbar display".to_string());
    }
    if previous.status_line_provider_labels != next.status_line_provider_labels {
        changes.push(format!("status line labels {:?}", next.status_line_provider_labels));
    }
    if previous.status_line_layout != next.status_line_layout {
        changes.push("status line layout".to_string());
    }
    if previous.notify_low_quota != next.notify_low_quota {
        changes.push(format!("low quota alerts {}", on_off(next.notify_low_quota)));
    }
    if previous.notify_read_failures != next.notify_read_failures {
        changes.push(format!("read failure alerts {}", on_off(next.notify_read_failures)));
    }
    if previous.notify_quota_resets != next.notify_quota_resets {
        changes.push(format!("reset alerts {}", on_off(next.notify_quota_resets)));
    }
    if previous.quota_disabled_providers != next.quota_disabled_providers {
        changes.push(format!(
            "quota tracked for [{}]",
            ProviderKind::ALL
                .into_iter()
                .filter(|provider| AppState::quota_tracked(next, *provider))
                .map(ProviderKind::key)
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if previous.dismissed_reset_notices != next.dismissed_reset_notices {
        changes
            .push(format!("{} restart note(s) acknowledged", next.dismissed_reset_notices.len()));
    }
    if previous.device_name != next.device_name {
        changes.push("this machine's name".to_string());
    }
    if previous.time_zone != next.time_zone {
        changes
            .push(format!("time zone {}", next.time_zone.as_deref().unwrap_or("follows Windows")));
    }
    if previous.shared_usage_folder != next.shared_usage_folder {
        changes.push(format!(
            "shared usage folder {}",
            if next.shared_usage_folder.is_some() { "set" } else { "cleared" }
        ));
    }
    changes
}

pub(crate) fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Records a change the settings dialog made. The status-line bridge reads the same file
/// on its next run, so a preference takes effect without the application telling it.
#[tauri::command]
pub(crate) async fn set_app_settings(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<SettingsView, String> {
    let previous = state.settings();
    let zone_changed = previous.time_zone != settings.time_zone;
    if zone_changed {
        // Refused before anything is saved, so an unknown name never reaches the file.
        if let Some(name) = settings.time_zone.as_deref() {
            crate::clock::resolve(name).map_err(|error| error.to_string())?;
        }
    }
    let quota_changed = previous.quota_disabled_providers != settings.quota_disabled_providers;
    let taskbar_changed = previous.taskbar_widget_enabled != settings.taskbar_widget_enabled;
    let display_changed = previous.taskbar_widget_display != settings.taskbar_widget_display;
    let theme_changed = previous.theme != settings.theme;
    let name_changed = previous.device_name != settings.device_name;
    let changes = settings_changes(&previous, &settings);
    let updated = state.update_settings(|current| *current = settings)?;
    // What changed rather than the record itself: the record carries this machine's
    // identity and its shared folder, and neither belongs in a file kept for diagnosis.
    log::write(if changes.is_empty() {
        "settings saved with no change".to_string()
    } else {
        format!("settings changed: {}", changes.join(", "))
    });
    if name_changed {
        let name = updated.device_name.clone().unwrap_or_else(settings::default_device_name);
        state.storage.record_local_device(&name).await.map_err(|error| {
            sanitize::sanitize_error(&error.to_string(), "This machine could not be renamed")
        })?;
    }
    if theme_changed {
        apply_theme(&app, updated.theme);
    }
    if zone_changed {
        // Every stored hour and day is keyed in the zone it was parsed in, so a new zone
        // takes a full parse: the history refresh sees the zone differ from the one the rows
        // were aggregated in and rebuilds them in one transaction.
        crate::clock::choose(updated.time_zone.as_deref()).map_err(|error| error.to_string())?;
        let refresh_app = app.clone();
        let refresh_state = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            refresh::refresh_history(&refresh_app, &refresh_state).await;
        });
    }
    if quota_changed {
        // Switching quota off has to clear it from the surfaces now rather than at the next
        // scheduled read, and switching it back on has nothing in memory to draw until
        // something reads it, so the snapshot is republished here and the read that fills a
        // returning provider runs behind it. Both are spawned: the publish lock can be held
        // by a scheduled history refresh for as long as the shared folder takes, and waiting
        // for it here would leave every settings card disabled until that finished.
        let publish_app = app.clone();
        let publish_state = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            refresh::republish(&publish_app, &publish_state).await;
        });
        let returning = ProviderKind::ALL.into_iter().filter(|provider| {
            !AppState::quota_tracked(&previous, *provider)
                && AppState::quota_tracked(&updated, *provider)
        });
        for provider in returning.collect::<Vec<_>>() {
            let app_handle = app.clone();
            let refresh_state = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                refresh::refresh_live_for_provider(&app_handle, &refresh_state, provider).await;
            });
        }
    }
    // Every window holds its own copy of the settings, read when it was created. The
    // dialog lives in one of them, so without this the others go on drawing the preference
    // they were started with — a quick panel still laid out at the density it was opened at
    // days ago.
    let _ = app.emit("settings-changed", SettingsView::of(updated.clone()));
    if taskbar_changed {
        set_taskbar_widget_visible(&app, updated.taskbar_widget_enabled);
    } else if display_changed && updated.taskbar_widget_enabled {
        // The placement loop would move it within two seconds; doing it here makes the
        // choice answer immediately, which is what a person changing it is watching for.
        schedule_taskbar_widget_placement(&app);
    }
    Ok(SettingsView::of(updated))
}

/// Whether Claude Code hands its quota to QuotaStation, for the card that offers to set
/// that up. Reading it touches only Claude Code's settings file, so it needs no refresh.
#[tauri::command]
pub(crate) fn get_claude_status_line() -> statusline::BridgeStatus {
    statusline::bridge_status()
}

/// Registers or removes QuotaStation as Claude Code's status-line command. Claude Code
/// hands the two quota windows to that command and to nothing else, so this is what turns
/// the seven-day window and both percentages on, without a credential or a network call.
#[tauri::command]
pub(crate) async fn set_claude_status_line(
    app: tauri::AppHandle,
    installed: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<statusline::BridgeStatus, String> {
    let result = if installed { statusline::install() } else { statusline::remove() };
    log::write(match &result {
        Ok(()) if installed => "status line installed into Claude Code's settings".to_string(),
        Ok(()) => "status line removed from Claude Code's settings".to_string(),
        Err(error) => format!("status line update failed: {error:#}"),
    });
    result.map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "Status line update failed")
    })?;
    // Claude Code writes the first reading on its next turn, so this refresh only picks up
    // one that is already there; the session watcher and the poll carry the rest.
    refresh::refresh_live_for_provider(&app, state.inner(), ProviderKind::Claude).await;
    Ok(statusline::bridge_status())
}

/// The status line a layout would draw, for the settings page to show before Claude Code
/// does. It comes from the bridge's own renderer, fed a sample session.
#[tauri::command]
pub(crate) fn preview_claude_status_line(
    layout: settings::StatusLineLayout,
    labels: settings::ProviderLabelStyle,
) -> String {
    statusline::preview(layout, labels)
}

/// Whether Claude Code tells QuotaStation that a turn has finished.
#[tauri::command]
pub(crate) fn get_claude_notifications() -> bool {
    notifications::installed()
}

/// Registers or removes QuotaStation as Claude Code's Stop hook, which is the only way to
/// learn that a turn finished: Claude Code's own notification channel reaches a handful of
/// terminals, and none of them are the ones this runs beside.
#[tauri::command]
pub(crate) fn set_claude_notifications(installed: bool) -> Result<bool, String> {
    let result = if installed { notifications::install() } else { notifications::remove() };
    log::write(match &result {
        Ok(()) if installed => "finished-turn hook installed into Claude Code".to_string(),
        Ok(()) => "finished-turn hook removed from Claude Code".to_string(),
        Err(error) => format!("finished-turn hook update failed: {error:#}"),
    });
    result.map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "Notification hook update failed")
    })?;
    Ok(notifications::installed())
}

/// The palettes every window should be drawing in right now.
#[tauri::command]
pub(crate) fn get_theme(state: State<'_, Arc<AppState>>) -> theme::ThemeSnapshot {
    theme::snapshot(state.settings().theme)
}

#[tauri::command]
pub(crate) async fn get_diagnostics(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<DiagnosticsSnapshot, String> {
    collect_diagnostics(&app, state.inner()).await
}

async fn collect_diagnostics(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<DiagnosticsSnapshot, String> {
    let mut acquisitions = Vec::new();
    let settings = state.settings();
    for provider in AppState::local_providers() {
        let rows = state.storage.load_acquisition_diagnostics(provider).await.map_err(|error| {
            sanitize::sanitize_error(&error.to_string(), "Diagnostics unavailable")
        })?;
        // A path nothing uses any more has no status worth reporting: leaving the last
        // failed quota read here would keep the panel asking to be looked at for a source
        // the user switched off.
        let tracked = AppState::quota_tracked(&settings, provider);
        let live_path = provider.live_path();
        acquisitions
            .extend(rows.into_iter().filter(|row| tracked || row.acquisition_path != live_path));
    }
    let retention =
        state.storage.load_retention_diagnostics().await.map_err(|error| {
            sanitize::sanitize_error(&error.to_string(), "Diagnostics unavailable")
        })?;
    let devices = state
        .storage
        .load_devices()
        .await
        .map_err(|error| sanitize::sanitize_error(&error.to_string(), "Diagnostics unavailable"))?
        .into_iter()
        .map(|device| DeviceDiagnostics {
            local: device.id == storage::LOCAL_DEVICE,
            id: device.id,
            display_name: device.display_name,
            last_import_at: device.last_import_at,
            restart_count: device.restart_count.unsigned_abs(),
        })
        .collect();
    Ok(DiagnosticsSnapshot {
        watcher: state.watcher_diagnostics.read().await.clone(),
        acquisitions,
        retention,
        shared_folder: state.shared_folder_diagnostics.read().await.clone(),
        devices,
        parser_revision: domain::CCUSAGE_REVISION.to_string(),
        pricing_catalog_revision: domain::PRICING_CATALOG_REVISION.to_string(),
        app_version: app.package_info().version.to_string(),
        build_commit: env!("QUOTASTATION_BUILD_COMMIT").to_string(),
        build_kind: build_kind(),
    })
}

/// Writes only the whitelisted diagnostic snapshot the user explicitly chose to export.
#[tauri::command]
pub(crate) async fn export_diagnostics(
    app: tauri::AppHandle,
    path: String,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let path = PathBuf::from(path);
    if path.extension().is_none_or(|extension| !extension.eq_ignore_ascii_case("json")) {
        return Err("Save the diagnostic export as a JSON file.".to_string());
    }
    log::write("diagnostics export requested");
    let diagnostics = collect_diagnostics(&app, state.inner()).await?;
    let mut providers = Vec::new();
    for provider in state.enabled_providers() {
        providers.push(state.read_snapshot(provider, Clone::clone).await);
    }
    diagnostic_export::DiagnosticExport::new(diagnostics, providers).write_to(&path)?;
    Ok(path.to_string_lossy().into_owned())
}
