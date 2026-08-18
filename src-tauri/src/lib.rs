mod alerts;
mod domain;
mod log;
mod providers;
mod refresh;
mod resets;
mod sanitize;
mod session_watcher;
mod settings;
mod storage;
mod summary;
mod taskbar;

use crate::settings::AppSettings;

use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex as StdMutex, OnceLock}, time::{Duration, Instant}};

use domain::{
    DiagnosticsSnapshot, ProviderSnapshot, UsageRangeSnapshot, WatcherDiagnostics,
    WorkspaceSnapshot,
};
use providers::{ProviderKind, claude::notifications, claude::statusline};
use storage::Storage;

/// Runs whichever Claude Code hook this process was started as, and reports whether it ran
/// one. Exposed so `main` can return before any window exists.
pub fn run_claude_hook() -> bool {
    statusline::run_bridge_if_requested() || notifications::run_hook_if_requested()
}
use tauri::{
    Manager, PhysicalPosition, State,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{Mutex, RwLock};

#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

/// The handle the taskbar click watch reaches the application through: it is called from a
/// system mouse hook, which is a bare C callback with nowhere to carry one.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

pub struct AppState {
    storage: Storage,
    snapshots: RwLock<BTreeMap<ProviderKind, ProviderSnapshot>>,
    refresh_publish_lock: Mutex<()>,
    live_refresh_lock: Mutex<()>,
    history_refresh_lock: Mutex<()>,
    watcher_diagnostics: RwLock<WatcherDiagnostics>,
    quick_panel_focus_lost_at: StdMutex<Option<Instant>>,
    quick_panel_toggled_at: StdMutex<Option<Instant>>,
    quick_panel_shown_at: StdMutex<Option<Instant>>,
    settings: StdMutex<AppSettings>,
    detected_providers: StdMutex<Vec<ProviderKind>>,
    settings_path: PathBuf,
}

impl AppState {
    fn settings(&self) -> AppSettings {
        self.settings.lock().map(|settings| settings.clone()).unwrap_or_default()
    }

    /// Applies a change and records it, so a preference the user expressed survives the
    /// next start whether it came from the tray or from the settings dialog.
    fn update_settings(&self, change: impl FnOnce(&mut AppSettings)) -> Result<AppSettings, String> {
        let mut settings = self.settings.lock().map_err(|_| "Settings unavailable.".to_string())?;
        let mut updated = settings.clone();
        change(&mut updated);
        settings::save(&self.settings_path, &updated)?;
        *settings = updated.clone();
        Ok(updated)
    }

    /// The providers currently on display, in the order every surface shows them.
    fn enabled_providers(&self) -> Vec<ProviderKind> {
        self.detected_providers.lock().map(|providers| providers.clone()).unwrap_or_default()
    }

    /// Which provider clients have left usage records on this machine. A client can be
    /// installed or signed in at any time, so this is re-checked with every full refresh
    /// rather than only at startup.
    fn detect_providers(&self) -> Vec<ProviderKind> {
        let detected: Vec<ProviderKind> =
            ProviderKind::ALL.into_iter().filter(|provider| provider.is_installed()).collect();
        if let Ok(mut providers) = self.detected_providers.lock() {
            providers.clone_from(&detected);
        }
        detected
    }

    async fn with_snapshot(&self, provider: ProviderKind, edit: impl FnOnce(&mut ProviderSnapshot)) {
        let mut snapshots = self.snapshots.write().await;
        edit(snapshots.entry(provider).or_insert_with(|| ProviderSnapshot::new(provider)));
    }

    async fn read_snapshot<T>(
        &self,
        provider: ProviderKind,
        read: impl FnOnce(&ProviderSnapshot) -> T,
    ) -> T {
        let mut snapshots = self.snapshots.write().await;
        read(snapshots.entry(provider).or_insert_with(|| ProviderSnapshot::new(provider)))
    }

    /// The payload every surface consumes. Derived state is resolved here so a snapshot
    /// never reaches the renderer with a status that disagrees with its own errors.
    async fn workspace_snapshot(&self) -> WorkspaceSnapshot {
        let snapshots = self.snapshots.read().await;
        let providers = self
            .enabled_providers()
            .into_iter()
            .map(|provider| {
                let mut snapshot = snapshots
                    .get(&provider)
                    .cloned()
                    .unwrap_or_else(|| ProviderSnapshot::new(provider));
                snapshot.resolve_derived_state();
                snapshot
            })
            .collect();
        WorkspaceSnapshot::new(providers)
    }
}

#[tauri::command]
async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<WorkspaceSnapshot, String> {
    Ok(state.workspace_snapshot().await)
}

#[tauri::command]
async fn get_usage_range(
    provider: ProviderKind,
    start_date: String,
    end_date: String,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageRangeSnapshot, String> {
    state
        .storage
        .load_usage_range(provider, &start_date, &end_date)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn refresh_now(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<WorkspaceSnapshot, String> {
    Ok(refresh::refresh_all(&app, state.inner()).await)
}

#[tauri::command]
fn open_dashboard(app: tauri::AppHandle) {
    if let Some(panel) = app.get_webview_window("quick-panel") {
        let _ = panel.hide();
    }
    show_main(&app);
}

/// Placement runs on a short loop so the widget follows taskbar changes. Repeating the
/// same failure every tick would bury every other message, so only changes are reported.
fn place_taskbar_widget(app: &tauri::AppHandle) {
    static LAST_ERROR: StdMutex<Option<String>> = StdMutex::new(None);
    let error = taskbar::place_widget(app).err();
    let Ok(mut last_error) = LAST_ERROR.lock() else { return };
    if *last_error != error {
        if let Some(message) = &error {
            log::write(format!("taskbar status placement: {message}"));
        }
        *last_error = error;
    }
}

fn set_taskbar_widget_visible(app: &tauri::AppHandle, visible: bool) {
    let state = app.state::<Arc<AppState>>();
    if let Err(error) = state.update_settings(|settings| settings.taskbar_widget_enabled = visible) {
        log::write(format!("failed to save application settings: {error}"));
    }
    if visible {
        // Placement comes first: it rebuilds the window when Explorer's taskbar took it
        // with it, and showing a window that no longer exists is what ends the process.
        place_taskbar_widget(app);
        if let Some(widget) = app.get_webview_window(&taskbar::widget_label()).filter(|_| taskbar::widget_is_live(app)) {
            let _ = widget.show();
        }
        // A low-level hook is called on the thread that installed it, so it has to be
        // installed on the one running the message loop.
        let _ = app.run_on_main_thread(taskbar::watch_widget_clicks);
    } else if let Some(widget) = app.get_webview_window(&taskbar::widget_label()).filter(|_| taskbar::widget_is_live(app)) {
        let _ = widget.hide();
    }
}

/// Which display's taskbar the user chose to host the status widget, for [`taskbar`].
///
/// The placement loop runs before the settings dialog has ever been opened and after the
/// window it belongs to is gone, so it reads the recorded choice rather than being told.
pub(crate) fn preferred_taskbar_display(app: &tauri::AppHandle) -> Option<String> {
    app.try_state::<Arc<AppState>>()?.settings().taskbar_widget_display
}

/// The displays the status can be shown on. Read live rather than stored: a monitor is
/// attached and detached while the application runs.
#[tauri::command]
fn get_taskbar_displays() -> Vec<taskbar::TaskbarDisplay> {
    taskbar::taskbar_displays()
}

#[tauri::command]
fn set_taskbar_widget_size(app: tauri::AppHandle, provider_count: u32) -> Result<(), String> {
    // A hidden widget still runs its renderer; resizing it must not bring it back.
    if !app.state::<Arc<AppState>>().settings().taskbar_widget_enabled {
        return Ok(());
    }
    taskbar::set_widget_size(&app, provider_count)
}

#[tauri::command]
fn get_app_settings(state: State<'_, Arc<AppState>>) -> AppSettings {
    state.settings()
}

/// Records a change the settings dialog made. The status-line bridge reads the same file
/// on its next run, so a preference takes effect without the application telling it.
#[tauri::command]
fn set_app_settings(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let previous = state.settings();
    let taskbar_changed = previous.taskbar_widget_enabled != settings.taskbar_widget_enabled;
    let display_changed = previous.taskbar_widget_display != settings.taskbar_widget_display;
    let updated = state.update_settings(|current| *current = settings)?;
    if taskbar_changed {
        set_taskbar_widget_visible(&app, updated.taskbar_widget_enabled);
    } else if display_changed && updated.taskbar_widget_enabled {
        // The placement loop would move it within two seconds; doing it here makes the
        // choice answer immediately, which is what a person changing it is watching for.
        place_taskbar_widget(&app);
    }
    Ok(updated)
}

/// Whether the activity log can be revealed without exposing its path to the renderer.
///
/// A built executable has no console: neither the application nor the status-line bridge
/// can report what it did anywhere a person could see, so both write to this file and the
/// diagnostics panel points at it.
#[tauri::command]
fn get_log_available() -> bool {
    log::log_path().is_some()
}

#[tauri::command]
fn reveal_log_file() -> Result<(), String> {
    let path = log::log_path().ok_or_else(|| "No application data directory.".to_string())?;
    // Selecting the file rather than opening it: the log is read with whatever the user
    // prefers, and a missing file still lands them in the right folder.
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Whether Claude Code hands its quota to QuotaStation, for the card that offers to set
/// that up. Reading it touches only Claude Code's settings file, so it needs no refresh.
#[tauri::command]
fn get_claude_status_line() -> statusline::BridgeStatus {
    statusline::bridge_status()
}

/// Registers or removes QuotaStation as Claude Code's status-line command. Claude Code
/// hands the two quota windows to that command and to nothing else, so this is what turns
/// the seven-day window and both percentages on, without a credential or a network call.
#[tauri::command]
async fn set_claude_status_line(
    app: tauri::AppHandle,
    installed: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<statusline::BridgeStatus, String> {
    let result = if installed { statusline::install() } else { statusline::remove() };
    result.map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "Status line update failed")
    })?;
    // Claude Code writes the first reading on its next turn, so this refresh only picks up
    // one that is already there; the session watcher and the poll carry the rest.
    refresh::refresh_live_for_provider(&app, state.inner(), ProviderKind::Claude).await;
    Ok(statusline::bridge_status())
}

/// Whether Claude Code tells QuotaStation that a turn has finished.
#[tauri::command]
fn get_claude_notifications() -> bool {
    notifications::installed()
}

/// Registers or removes QuotaStation as Claude Code's Stop hook, which is the only way to
/// learn that a turn finished: Claude Code's own notification channel reaches a handful of
/// terminals, and none of them are the ones this runs beside.
#[tauri::command]
fn set_claude_notifications(installed: bool) -> Result<bool, String> {
    let result = if installed { notifications::install() } else { notifications::remove() };
    result.map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "Notification hook update failed")
    })?;
    Ok(notifications::installed())
}

/// Raises the desktop notification a finished Claude Code turn left behind.
///
/// The hook process cannot show one itself — it has no window, no event loop, and a few
/// milliseconds to live — so it writes an event and this picks it up. Polling one path is
/// what that costs; the alternative is a filesystem watcher for a file written a handful of
/// times an hour.
fn watch_for_finished_turns(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        loop {
            ticks.tick().await;
            let Some(event) = notifications::take_pending(jiff::Timestamp::now().as_second())
            else {
                continue;
            };
            // The title says which event this is, the same way the quota notifications do.
            // Windows already prints the application's name above it, so spending the title
            // on "QuotaStation" left every notification looking alike in the action centre.
            let body = match event.project {
                Some(project) => project,
                None => "A turn has ended".to_string(),
            };
            alerts::raise(&app, "Claude Code finished responding", &body);
        }
    });
}

/// Which build is running, told apart the way the machine can tell them apart: the compiler
/// knows debug from release, and an installer leaves an uninstaller beside the executable
/// while a portable copy does not.
fn build_kind() -> String {
    if cfg!(debug_assertions) {
        return "debug".to_string();
    }
    let installed = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|dir| dir.join("uninstall.exe")))
        .is_some_and(|uninstaller| uninstaller.exists());
    if installed { "release, installed".to_string() } else { "release, portable".to_string() }
}

#[tauri::command]
async fn get_diagnostics(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<DiagnosticsSnapshot, String> {
    let mut acquisitions = Vec::new();
    for provider in state.enabled_providers() {
        acquisitions.extend(
            state
                .storage
                .load_acquisition_diagnostics(provider)
                .await
                .map_err(|error| {
                    sanitize::sanitize_error(&error.to_string(), "Diagnostics unavailable")
                })?,
        );
    }
    let retention = state
        .storage
        .load_retention_diagnostics()
        .await
        .map_err(|error| {
            sanitize::sanitize_error(&error.to_string(), "Diagnostics unavailable")
        })?;
    Ok(DiagnosticsSnapshot {
        watcher: state.watcher_diagnostics.read().await.clone(),
        acquisitions,
        retention,
        parser_revision: domain::CCUSAGE_REVISION.to_string(),
        pricing_catalog_revision: domain::PRICING_CATALOG_REVISION.to_string(),
        app_version: app.package_info().version.to_string(),
        build_commit: env!("QUOTASTATION_BUILD_COMMIT").to_string(),
        build_kind: build_kind(),
    })
}

/// Codex logs the server's rate-limit answer alongside its own token counts, which
/// reaches back further than this database and covers every stretch when QuotaStation was
/// closed. Replaying it on startup is what makes the restart history complete rather than
/// starting from whenever this feature was installed.
async fn backfill_resets(state: &Arc<AppState>) -> anyhow::Result<()> {
    for provider in state.enabled_providers() {
        let since = state.storage.reset_backfill_start(provider).await?;
        let observations = providers::read_observations(provider, since).await?;
        if observations.is_empty() {
            continue;
        }
        let scanned_at = jiff::Timestamp::now().to_string();
        state
            .storage
            .backfill_resets(provider, &observations, &scanned_at)
            .await?;
    }
    Ok(())
}

/// Windows refuses a raise request from a process that does not own the foreground, which
/// a tray menu click does not, so a window that is merely behind another one stays there
/// after `set_focus`. Briefly claiming always-on-top is what actually brings it forward.
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        let _ = window.set_focus();
        let _ = window.set_always_on_top(false);
    }
}

/// The panel shows one column per provider, so its width follows how many are enabled.
/// Sizing it as it opens keeps the edge anchoring below working from the real size.
///
/// This is the width the columns are laid out at, so a scaled display is given the scaled
/// window: 390 device pixels left a 125% display 312 layout pixels to draw a 390-pixel
/// column in, and the reflow made the panel taller than the height the renderer had
/// already measured at the width the window opened with.
const QUICK_PANEL_COLUMN_WIDTH: f64 = 390.0;
/// Only what the window opens at before the renderer has measured anything. Every height
/// after the first render comes from [`set_quick_panel_height`].
const QUICK_PANEL_HEIGHT: u32 = 730;
/// The gap the panel keeps from every edge of the work area, shared by the placement and
/// the growth below so a panel that grows stops exactly where one that opens would.
const QUICK_PANEL_MARGIN: f64 = 12.0;

/// What the window rect holds that the page is not drawn in.
///
/// An undecorated window with a shadow keeps an invisible resize frame, so its window rect
/// is larger than its content area — 18 x 10 physical pixels at 125% here. `outer_size`,
/// `outer_position` and the work area are all in window-rect coordinates while `set_size`
/// takes a content size, so every placement below stays in window-rect units and converts
/// exactly once, in [`resize_quick_panel`]. Reading one and writing the other grew the
/// window by this frame on every open, and the content, which had not changed, kept the
/// height it was measured at — leaving a band of bare background around the card that read
/// as a second panel behind it.
fn quick_panel_frame(panel: &tauri::WebviewWindow) -> tauri::PhysicalSize<u32> {
    let Ok(outer) = panel.outer_size() else { return tauri::PhysicalSize::new(0, 0) };
    let inner = panel.inner_size().unwrap_or(outer);
    tauri::PhysicalSize::new(
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    )
}

fn resize_quick_panel(
    panel: &tauri::WebviewWindow,
    frame: tauri::PhysicalSize<u32>,
    outer: tauri::PhysicalSize<u32>,
) {
    let _ = panel.set_size(tauri::PhysicalSize::new(
        outer.width.saturating_sub(frame.width).max(1),
        outer.height.saturating_sub(frame.height).max(1),
    ));
}

/// The window rect the panel needs for `providers` columns, given the height it already holds.
fn quick_panel_size(
    providers: usize,
    height: u32,
    scale_factor: f64,
    frame: tauri::PhysicalSize<u32>,
) -> tauri::PhysicalSize<u32> {
    let columns = QUICK_PANEL_COLUMN_WIDTH * providers.clamp(1, 2) as f64 * scale_factor.max(1.0);
    tauri::PhysicalSize::new((columns.round() as u32).saturating_add(frame.width), height)
}

/// Where the panel sits once the renderer reports a different content height.
///
/// The bottom edge is the fixed one: the placement anchored it beside the tray, so the
/// panel grows away from that edge rather than sliding out from under the pointer. Content
/// taller than the work area is clamped to it, and the panel scrolls its own contents from
/// there — there is nowhere left to grow.
fn quick_panel_growth(
    work_area: tauri::PhysicalRect<i32, u32>,
    position: PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    requested_height: u32,
) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    let margin = QUICK_PANEL_MARGIN as i32;
    let available = (work_area.size.height as f64 - QUICK_PANEL_MARGIN * 2.0).max(1.0) as u32;
    let height = requested_height.clamp(1, available);
    let top_limit = work_area.position.y + margin;
    let bottom_limit = work_area.position.y + work_area.size.height as i32 - margin;
    let bottom = (position.y + size.height as i32).min(bottom_limit);
    let y = (bottom - height as i32).max(top_limit);
    (
        PhysicalPosition::new(position.x, y),
        tauri::PhysicalSize::new(size.width, height),
    )
}

/// The height the renderer measured, in CSS pixels, for a window that has no frame to
/// trim it to its contents.
#[tauri::command]
fn set_quick_panel_height(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    let Some(panel) = app.get_webview_window("quick-panel") else { return Ok(()) };
    if !height.is_finite() || height <= 0.0 {
        return Ok(());
    }
    let scale_factor = panel.scale_factor().map_err(|error| error.to_string())?;
    let frame = quick_panel_frame(&panel);
    let size = panel.outer_size().map_err(|error| error.to_string())?;
    let position = panel.outer_position().map_err(|error| error.to_string())?;
    let requested = ((height * scale_factor).round().clamp(1.0, u32::MAX as f64) as u32)
        .saturating_add(frame.height);
    let work_area = panel
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| *monitor.work_area());
    let (next_position, next_size) = match work_area {
        Some(work_area) => quick_panel_growth(work_area, position, size, requested),
        // Without a monitor there is nothing to clamp against, so the request stands and
        // the bottom edge still holds.
        None => (
            PhysicalPosition::new(position.x, position.y + size.height as i32 - requested as i32),
            tauri::PhysicalSize::new(size.width, requested),
        ),
    };
    if next_size == size && next_position == position {
        return Ok(());
    }
    resize_quick_panel(&panel, frame, next_size);
    panel.set_position(next_position).map_err(|error| error.to_string())?;
    Ok(())
}

fn quick_panel_placement(
    work_area: tauri::PhysicalRect<i32, u32>,
    tray_position: PhysicalPosition<f64>,
    tray_size: tauri::PhysicalSize<f64>,
    requested_size: tauri::PhysicalSize<u32>,
) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    let margin = QUICK_PANEL_MARGIN;
    let left = work_area.position.x as f64;
    let top = work_area.position.y as f64;
    let right = left + work_area.size.width as f64;
    let bottom = top + work_area.size.height as f64;
    let available_width = (work_area.size.width as f64 - margin * 2.0).max(1.0);
    let available_height = (work_area.size.height as f64 - margin * 2.0).max(1.0);
    let panel_width = (requested_size.width as f64).min(available_width);
    let panel_height = (requested_size.height as f64).min(available_height);
    let panel_size =
        tauri::PhysicalSize::new(panel_width.round() as u32, panel_height.round() as u32);
    let anchor = PhysicalPosition::new(
        tray_position.x + tray_size.width / 2.0,
        tray_position.y + tray_size.height / 2.0,
    );
    let nearest = [
        (anchor.x - left, "left"),
        (right - anchor.x, "right"),
        (anchor.y - top, "top"),
        (bottom - anchor.y, "bottom"),
    ]
    .into_iter()
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, edge)| edge)
    .unwrap_or("bottom");
    let max_x = (right - panel_width - margin).max(left + margin);
    let max_y = (bottom - panel_height - margin).max(top + margin);
    let clamp_x = |value: f64| value.clamp(left + margin, max_x);
    let clamp_y = |value: f64| value.clamp(top + margin, max_y);
    let (x, y) = match nearest {
        "top" => (
            clamp_x(tray_position.x + tray_size.width - panel_width),
            clamp_y(tray_position.y + tray_size.height + margin),
        ),
        "left" => (
            clamp_x(tray_position.x + tray_size.width + margin),
            clamp_y(tray_position.y + tray_size.height - panel_height),
        ),
        "right" => (
            clamp_x(tray_position.x - panel_width - margin),
            clamp_y(tray_position.y + tray_size.height - panel_height),
        ),
        _ => (
            clamp_x(tray_position.x + tray_size.width - panel_width),
            clamp_y(tray_position.y - panel_height - margin),
        ),
    };
    (PhysicalPosition::new(x.round() as i32, y.round() as i32), panel_size)
}

/// Shows or hides the panel beside `anchor`, given in physical screen coordinates: the tray
/// icon for a click on the tray, the docked widget for a click on the taskbar status. Both
/// open the one panel — a second panel for the second surface would be the same readings
/// drawn twice.
///
/// Reports whether the panel is now open.
fn toggle_quick_panel_beside(
    app: &tauri::AppHandle,
    anchor_position: PhysicalPosition<f64>,
    anchor_size: tauri::PhysicalSize<f64>,
) -> bool {
    let Some(panel) = app.get_webview_window("quick-panel") else { return false };
    let state = app.state::<Arc<AppState>>();
    // One click opens the panel once. The tray icon and the taskbar status sit in the same
    // corner and there is one panel between them, so a second request arriving on the heels
    // of the first is the same click reaching a second path — obeying it moved the panel to
    // the other anchor, which read as a second window replacing the first.
    if let Ok(mut toggled_at) = state.quick_panel_toggled_at.lock() {
        if toggled_at.is_some_and(|at| at.elapsed() < Duration::from_millis(300)) {
            return panel.is_visible().unwrap_or(false);
        }
        *toggled_at = Some(Instant::now());
    }
    // The renderer has already sized the window to its contents, so the panel opens at the
    // height it currently holds rather than at the height it was configured with.
    let frame = quick_panel_frame(&panel);
    let current_height = panel.outer_size().map(|size| size.height).unwrap_or(QUICK_PANEL_HEIGHT);
    let requested_size = quick_panel_size(
        state.enabled_providers().len(),
        current_height,
        panel.scale_factor().unwrap_or(1.0),
        frame,
    );
    if let Ok(mut focus_lost_at) = state.quick_panel_focus_lost_at.lock()
        && focus_lost_at.is_some_and(|lost_at| lost_at.elapsed() < Duration::from_millis(500))
    {
        *focus_lost_at = None;
        return false;
    }
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return false;
    }

    let centre = (
        anchor_position.x + anchor_size.width / 2.0,
        anchor_position.y + anchor_size.height / 2.0,
    );
    let monitor = app.monitor_from_point(centre.0, centre.1).ok().flatten();
    let (x, y) = if let Some(monitor) = monitor {
        let (position, fitted_size) = quick_panel_placement(
            *monitor.work_area(),
            anchor_position,
            anchor_size,
            requested_size,
        );
        resize_quick_panel(&panel, frame, fitted_size);
        (position.x as f64, position.y as f64)
    } else {
        resize_quick_panel(&panel, frame, requested_size);
        (
            anchor_position.x - requested_size.width as f64,
            anchor_position.y - requested_size.height as f64,
        )
    };
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    if let Ok(mut shown_at) = state.quick_panel_shown_at.lock() {
        *shown_at = Some(Instant::now());
    }
    let _ = panel.show();
    let _ = panel.set_focus();
    true
}

/// The tray reports its icon in whichever unit the platform uses, so the click position —
/// which is already physical — is what identifies the monitor whose scale converts it.
fn toggle_quick_panel(app: &tauri::AppHandle, click: PhysicalPosition<f64>, tray_rect: tauri::Rect) {
    let scale_factor = app
        .monitor_from_point(click.x, click.y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    toggle_quick_panel_beside(
        app,
        tray_rect.position.to_physical(scale_factor),
        tray_rect.size.to_physical(scale_factor),
    );
}

/// Opens the panel above the taskbar status, anchored to the widget rather than to the tray
/// icon.
///
/// Called from the click watch in [`taskbar`], which runs inside a low-level mouse hook, so
/// the work is queued onto the main thread rather than done there: a hook that takes its
/// time is a hook the system stops calling.
pub(crate) fn open_quick_panel_from_taskbar() {
    let Some(app) = APP.get().cloned() else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Ok((position, size)) = taskbar::widget_screen_rect(&handle) else { return };
        if toggle_quick_panel_beside(&handle, position, size) {
            let _ = taskbar::raise_window(&handle, "quick-panel");
        }
    });
}

#[cfg(test)]
mod quick_panel_tests {
    use super::*;

    fn placement(
        width: u32,
        height: u32,
        tray_x: f64,
        tray_y: f64,
    ) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
        quick_panel_placement(
            tauri::PhysicalRect {
                position: PhysicalPosition::new(0, 0),
                size: tauri::PhysicalSize::new(width, height),
            },
            PhysicalPosition::new(tray_x, tray_y),
            tauri::PhysicalSize::new(40.0, 40.0),
            tauri::PhysicalSize::new(780, 730),
        )
    }

    fn work_area(height: u32) -> tauri::PhysicalRect<i32, u32> {
        tauri::PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: tauri::PhysicalSize::new(1280, height),
        }
    }

    #[test]
    fn a_shorter_panel_keeps_its_bottom_edge_and_a_taller_one_grows_upwards() {
        let position = PhysicalPosition::new(400, 300);
        let size = tauri::PhysicalSize::new(390, 400);
        let bottom = position.y + size.height as i32;
        for requested in [200, 400, 620] {
            let (next_position, next_size) = quick_panel_growth(work_area(1000), position, size, requested);
            assert_eq!(next_size.height, requested);
            assert_eq!(next_size.width, size.width, "only the height follows the contents");
            assert_eq!(next_position.x, position.x);
            assert_eq!(next_position.y + next_size.height as i32, bottom);
        }
    }

    #[test]
    fn a_panel_taller_than_the_work_area_is_clamped_inside_it() {
        let (position, size) = quick_panel_growth(
            work_area(720),
            PhysicalPosition::new(400, 300),
            tauri::PhysicalSize::new(390, 400),
            2_000,
        );
        assert!(position.y >= 12, "the top margin is kept");
        assert!(position.y + size.height as i32 <= 720 - 12, "so is the bottom one");
    }

    #[test]
    fn a_column_is_reserved_in_layout_pixels_whatever_the_display_scales_by() {
        let frame = tauri::PhysicalSize::new(18, 10);
        assert_eq!(quick_panel_size(2, 600, 1.0, frame).width, 780 + 18);
        assert_eq!(quick_panel_size(2, 600, 1.25, frame).width, 975 + 18);
        assert_eq!(quick_panel_size(1, 600, 1.0, frame).width, 390 + 18);
        assert_eq!(
            quick_panel_size(3, 600, 1.0, frame).width,
            780 + 18,
            "two columns is the widest the panel goes"
        );
        assert_eq!(quick_panel_size(2, 600, 1.0, frame).height, 600, "the height is passed through");
    }

    #[test]
    fn quick_panel_fits_small_and_common_displays() {
        for (width, height) in [(800, 600), (1280, 720), (1366, 768)] {
            for tray_x in [0.0, width as f64 - 40.0] {
                let (position, size) = placement(width, height, tray_x, height as f64 - 40.0);
                assert!(position.x >= 0 && position.y >= 0);
                assert!(position.x as u32 + size.width <= width);
                assert!(position.y as u32 + size.height <= height);
            }
        }
    }
}

#[cfg(windows)]
fn write_desktop_shortcut(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let shortcut_path = app
        .path()
        .desktop_dir()
        .map_err(|error| error.to_string())?
        .join("QuotaStation.lnk");
    let mut shortcut = mslnk::ShellLink::new(&executable).map_err(|error| error.to_string())?;
    if let Some(working_directory) = executable.parent() {
        shortcut.set_working_dir(Some(working_directory.to_string_lossy().into_owned()));
    }
    shortcut.set_icon_location(Some(executable.to_string_lossy().into_owned()));
    shortcut.set_name(Some("QuotaStation".to_string()));
    shortcut.create_lnk(&shortcut_path).map_err(|error| error.to_string())?;
    Ok(shortcut_path)
}

#[cfg(not(windows))]
fn write_desktop_shortcut(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Err("Desktop shortcuts are currently supported on Windows only.".to_string())
}

/// Whether Windows starts QuotaStation on sign-in. The plugin owns the registration, so
/// this reports what it holds rather than a copy kept in the settings file.
#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "Start-with-Windows update failed")
    })?;
    Ok(manager.is_enabled().unwrap_or(enabled))
}

/// Puts a shortcut on the desktop. The location is the user's own desktop, so the path is
/// neither reported back nor worth reporting.
#[tauri::command]
fn create_desktop_shortcut(app: tauri::AppHandle) -> Result<(), String> {
    write_desktop_shortcut(&app)
        .map(|_| ())
        .map_err(|error| sanitize::sanitize_error(&error, "Desktop shortcut creation failed"))
}

/// The tray menu carries what has to work when no window is open: showing the dashboard,
/// a manual refresh, and quitting. Every preference lives in the settings dialog instead,
/// so a setting is changed in one place rather than in whichever surface found it first.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show QuotaStation", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let separator_before_quit = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &refresh, &separator_before_quit, &quit])?;
    let icon = app.default_window_icon().cloned().expect("application icon must be configured");
    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "refresh" => {
                let app = app.clone();
                let state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move { refresh::refresh_all(&app, &state).await; });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { position, rect, button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_quick_panel(tray.app_handle(), position, rect);
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            log::write(format!(
                "application started, version {}, status line {}",
                app.package_info().version,
                match statusline::bridge_status().installed {
                    true => "installed",
                    false => "not installed",
                }
            ));
            let app_data_dir = app.path().app_data_dir()?;
            let database_path = app_data_dir.join("quotastation.db");
            let settings_path = app_data_dir.join("settings.json");
            let settings = settings::load(&settings_path);
            let storage = tauri::async_runtime::block_on(Storage::open(&database_path))
                .map_err(|error| error.to_string())?;
            if let Err(error) = tauri::async_runtime::block_on(storage.run_retention_if_due()) {
                log::write(format!("normalized data retention failed: {error:#}"));
            }
            let mut snapshots = BTreeMap::new();
            for provider in ProviderKind::ALL {
                if !provider.is_installed() {
                    continue;
                }
                let snapshot = tauri::async_runtime::block_on(storage.load_snapshot(provider))
                    .unwrap_or_else(|_| ProviderSnapshot::new(provider));
                snapshots.insert(provider, snapshot);
            }
            let state = Arc::new(AppState {
                storage,
                snapshots: RwLock::new(snapshots),
                refresh_publish_lock: Mutex::new(()),
                live_refresh_lock: Mutex::new(()),
                history_refresh_lock: Mutex::new(()),
                watcher_diagnostics: RwLock::new(WatcherDiagnostics::default()),
                quick_panel_focus_lost_at: StdMutex::new(None),
                quick_panel_toggled_at: StdMutex::new(None),
                quick_panel_shown_at: StdMutex::new(None),
                settings: StdMutex::new(settings),
                detected_providers: StdMutex::new(Vec::new()),
                settings_path,
            });
            state.detect_providers();
            app.manage(state.clone());
            let _ = APP.set(app.handle().clone());
            build_tray(app)?;
            watch_for_finished_turns(app.handle().clone());
            if state.settings().taskbar_widget_enabled {
                set_taskbar_widget_visible(app.handle(), true);
            }
            if session_watcher::start(app.handle().clone(), state.clone()).is_err() {
                tauri::async_runtime::block_on(async {
                    let mut diagnostics = state.watcher_diagnostics.write().await;
                    diagnostics.status = "unavailable".to_string();
                    diagnostics.error = Some("Session watching is unavailable; periodic reconciliation remains active.".to_string());
                });
            }
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                refresh::refresh_all(&app_handle, &state).await;
            });
            let backfill_state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = backfill_resets(&backfill_state).await {
                    log::write(format!("quota reset backfill failed: {error:#}"));
                }
            });
            let retention_storage = app.state::<Arc<AppState>>().storage.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(error) = retention_storage.run_retention_if_due().await {
                        log::write(format!("normalized data retention failed: {error:#}"));
                    }
                }
            });
            let app_handle = app.handle().clone();
            let taskbar_state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(2));
                loop {
                    interval.tick().await;
                    if taskbar_state.settings().taskbar_widget_enabled {
                        place_taskbar_widget(&app_handle);
                    }
                }
            });
            // Each provider polls on its own interval: a local process tolerates a
            // frequent read, a rate-limited remote endpoint does not.
            for provider in ProviderKind::ALL {
                let app_handle = app.handle().clone();
                let live_state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(provider.live_refresh_interval());
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        refresh::refresh_live_for_provider(&app_handle, &live_state, provider).await;
                    }
                });
            }
            let app_handle = app.handle().clone();
            let history_state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(900));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    refresh::refresh_history(&app_handle, &history_state).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_usage_range,
            refresh_now,
            get_diagnostics,
            get_log_available,
            reveal_log_file,
            get_claude_status_line,
            set_claude_status_line,
            get_claude_notifications,
            set_claude_notifications,
            open_dashboard,
            set_taskbar_widget_size,
            get_taskbar_displays,
            set_quick_panel_height,
            get_app_settings,
            set_app_settings,
            get_autostart,
            set_autostart,
            create_desktop_shortcut
        ])
        .on_window_event(|window, event| {
            if window.label() == "quick-panel" && matches!(event, tauri::WindowEvent::Focused(false)) {
                let state = window.state::<Arc<AppState>>();
                // A panel opened by a click on somebody else's window is told it lost focus
                // before it ever had it — the click belongs to that window, and Windows hands
                // the foreground back. Dismissing on that is dismissing the panel the click
                // just asked for, which looks like the click doing nothing at all.
                let just_shown = state
                    .quick_panel_shown_at
                    .lock()
                    .ok()
                    .and_then(|shown_at| *shown_at)
                    .is_some_and(|shown_at| shown_at.elapsed() < Duration::from_millis(400));
                if just_shown {
                    return;
                }
                if let Ok(mut focus_lost_at) = state.quick_panel_focus_lost_at.lock() {
                    *focus_lost_at = Some(Instant::now());
                }
                let _ = window.hide();
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running QuotaStation");
}
