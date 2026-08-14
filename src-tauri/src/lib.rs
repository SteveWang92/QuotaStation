mod domain;
mod log;
mod providers;
mod refresh;
mod resets;
mod sanitize;
mod session_watcher;
mod storage;
mod taskbar;

use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex as StdMutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

use domain::{
    DiagnosticsSnapshot, ProviderSnapshot, UsageRangeSnapshot, WatcherDiagnostics,
    WorkspaceSnapshot,
};
use providers::{ProviderKind, claude::statusline};
use storage::Storage;

/// Runs Claude Code's status-line command when this process was started as one, and reports
/// whether it did. Exposed so `main` can return before any window exists.
pub fn run_claude_status_line() -> bool {
    statusline::run_bridge_if_requested()
}
use tauri::{
    Manager, PhysicalPosition, State,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{Mutex, RwLock};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    #[serde(default = "default_taskbar_widget_enabled")]
    taskbar_widget_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            taskbar_widget_enabled: default_taskbar_widget_enabled(),
        }
    }
}

fn default_taskbar_widget_enabled() -> bool { true }

fn load_settings(path: &std::path::Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_settings(state: &AppState) -> Result<(), String> {
    let settings = AppSettings {
        taskbar_widget_enabled: state.taskbar_widget_enabled.load(Ordering::Relaxed),
    };
    if let Some(parent) = state.settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    std::fs::write(&state.settings_path, content).map_err(|error| error.to_string())
}

#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

pub struct AppState {
    storage: Storage,
    snapshots: RwLock<BTreeMap<ProviderKind, ProviderSnapshot>>,
    live_refresh_lock: Mutex<()>,
    history_refresh_lock: Mutex<()>,
    watcher_diagnostics: RwLock<WatcherDiagnostics>,
    quick_panel_focus_lost_at: StdMutex<Option<Instant>>,
    taskbar_widget_enabled: AtomicBool,
    detected_providers: StdMutex<Vec<ProviderKind>>,
    settings_path: PathBuf,
}

impl AppState {
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
            eprintln!("taskbar status placement: {message}");
        }
        *last_error = error;
    }
}

fn set_taskbar_widget_visible(app: &tauri::AppHandle, visible: bool) {
    let state = app.state::<Arc<AppState>>();
    state.taskbar_widget_enabled.store(visible, Ordering::Relaxed);
    if let Err(error) = save_settings(&state) {
        eprintln!("failed to save application settings: {error}");
    }
    if let Some(widget) = app.get_webview_window("taskbar-widget") {
        if visible {
            let _ = widget.show();
            place_taskbar_widget(app);
        } else {
            let _ = widget.hide();
        }
    }
}

/// Restores the widget to its own size and place after the renderer mounts. The size is a
/// constant now, so this exists to re-dock the window rather than to follow the content.
#[tauri::command]
fn set_taskbar_widget_size(app: tauri::AppHandle) -> Result<(), String> {
    // A hidden widget still runs its renderer; resizing it must not bring it back.
    if !app.state::<Arc<AppState>>().taskbar_widget_enabled.load(Ordering::Relaxed) {
        return Ok(());
    }
    taskbar::set_widget_size(&app)
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

#[tauri::command]
async fn get_diagnostics(state: State<'_, Arc<AppState>>) -> Result<DiagnosticsSnapshot, String> {
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
const QUICK_PANEL_COLUMN_WIDTH: u32 = 390;
const QUICK_PANEL_HEIGHT: u32 = 730;

fn quick_panel_size(providers: usize) -> tauri::PhysicalSize<u32> {
    tauri::PhysicalSize::new(
        QUICK_PANEL_COLUMN_WIDTH * providers.clamp(1, 2) as u32,
        QUICK_PANEL_HEIGHT,
    )
}

fn quick_panel_placement(
    work_area: tauri::PhysicalRect<i32, u32>,
    tray_position: PhysicalPosition<f64>,
    tray_size: tauri::PhysicalSize<f64>,
    requested_size: tauri::PhysicalSize<u32>,
) -> (PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    let margin = 12.0;
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

fn toggle_quick_panel(app: &tauri::AppHandle, click: PhysicalPosition<f64>, tray_rect: tauri::Rect) {
    let Some(panel) = app.get_webview_window("quick-panel") else { return };
    let state = app.state::<Arc<AppState>>();
    let requested_size = quick_panel_size(state.enabled_providers().len());
    if let Ok(mut focus_lost_at) = state.quick_panel_focus_lost_at.lock()
        && focus_lost_at.is_some_and(|lost_at| lost_at.elapsed() < Duration::from_millis(500))
    {
        *focus_lost_at = None;
        return;
    }
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }

    let monitor = app.monitor_from_point(click.x, click.y).ok().flatten();
    let (x, y) = if let Some(monitor) = monitor {
        let scale_factor = monitor.scale_factor();
        let tray_position = tray_rect.position.to_physical::<f64>(scale_factor);
        let tray_size = tray_rect.size.to_physical::<f64>(scale_factor);
        let (position, fitted_size) =
            quick_panel_placement(*monitor.work_area(), tray_position, tray_size, requested_size);
        let _ = panel.set_size(fitted_size);
        (position.x as f64, position.y as f64)
    } else {
        let _ = panel.set_size(requested_size);
        (
            click.x - requested_size.width as f64,
            click.y - requested_size.height as f64,
        )
    };
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = panel.show();
    let _ = panel.set_focus();
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
fn create_desktop_shortcut(app: &tauri::AppHandle) -> Result<PathBuf, String> {
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
    shortcut.set_name(Some("QuotaStation".to_string()));
    shortcut.create_lnk(&shortcut_path).map_err(|error| error.to_string())?;
    Ok(shortcut_path)
}

#[cfg(not(windows))]
fn create_desktop_shortcut(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Err("Desktop shortcuts are currently supported on Windows only.".to_string())
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show QuotaStation", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start with Windows",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let desktop_shortcut = MenuItem::with_id(
        app,
        "desktop_shortcut",
        "Create desktop shortcut",
        true,
        None::<&str>,
    )?;
    let taskbar_widget_enabled = app
        .state::<Arc<AppState>>()
        .taskbar_widget_enabled
        .load(Ordering::Relaxed);
    let taskbar_widget = CheckMenuItem::with_id(
        app,
        "taskbar_widget",
        "Show taskbar status",
        true,
        taskbar_widget_enabled,
        None::<&str>,
    )?;
    let separator_before_quit = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &refresh,
            &separator,
            &autostart,
            &desktop_shortcut,
            &taskbar_widget,
            &separator_before_quit,
            &quit,
        ],
    )?;
    let icon = app.default_window_icon().cloned().expect("application icon must be configured");
    let autostart_menu_item = autostart.clone();
    let taskbar_widget_menu_item = taskbar_widget.clone();
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
            "autostart" => {
                let manager = app.autolaunch();
                let enabled = manager.is_enabled().unwrap_or(false);
                let result = if enabled { manager.disable() } else { manager.enable() };
                match result {
                    Ok(()) => {
                        let _ = autostart_menu_item.set_checked(!enabled);
                    }
                    Err(error) => {
                        eprintln!("failed to update start-with-Windows setting: {error}");
                        let _ = autostart_menu_item.set_checked(enabled);
                    }
                }
            }
            "desktop_shortcut" => match create_desktop_shortcut(app) {
                Ok(path) => eprintln!("desktop shortcut created at {}", path.display()),
                Err(error) => eprintln!("failed to create desktop shortcut: {error}"),
            },
            "taskbar_widget" => {
                let state = app.state::<Arc<AppState>>();
                let enabled = !state.taskbar_widget_enabled.load(Ordering::Relaxed);
                set_taskbar_widget_visible(app, enabled);
                let _ = taskbar_widget_menu_item.set_checked(enabled);
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
            let settings = load_settings(&settings_path);
            let storage = tauri::async_runtime::block_on(Storage::open(&database_path))
                .map_err(|error| error.to_string())?;
            if let Err(error) = tauri::async_runtime::block_on(storage.run_retention_if_due()) {
                eprintln!("normalized data retention failed: {error:#}");
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
                live_refresh_lock: Mutex::new(()),
                history_refresh_lock: Mutex::new(()),
                watcher_diagnostics: RwLock::new(WatcherDiagnostics::default()),
                quick_panel_focus_lost_at: StdMutex::new(None),
                taskbar_widget_enabled: AtomicBool::new(settings.taskbar_widget_enabled),
                detected_providers: StdMutex::new(Vec::new()),
                settings_path,
            });
            state.detect_providers();
            app.manage(state.clone());
            build_tray(app)?;
            if state.taskbar_widget_enabled.load(Ordering::Relaxed) {
                set_taskbar_widget_visible(app.handle(), true);
            }
            if session_watcher::start(app.handle().clone(), state.clone()).is_err() {
                tauri::async_runtime::block_on(async {
                    let mut diagnostics = state.watcher_diagnostics.write().await;
                    diagnostics.status = "unavailable".to_string();
                    diagnostics.error = Some("Codex session watching is unavailable; periodic reconciliation remains active.".to_string());
                });
            }
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                refresh::refresh_all(&app_handle, &state).await;
            });
            let backfill_state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = backfill_resets(&backfill_state).await {
                    eprintln!("quota reset backfill failed: {error:#}");
                }
            });
            let app_handle = app.handle().clone();
            let taskbar_state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(2));
                loop {
                    interval.tick().await;
                    if taskbar_state.taskbar_widget_enabled.load(Ordering::Relaxed) {
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
            open_dashboard,
            set_taskbar_widget_size
        ])
        .on_window_event(|window, event| {
            if window.label() == "quick-panel" && matches!(event, tauri::WindowEvent::Focused(false)) {
                let state = window.state::<Arc<AppState>>();
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
