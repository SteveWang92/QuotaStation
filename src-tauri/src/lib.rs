mod domain;
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
use providers::ProviderKind;
use storage::Storage;
use tauri::{
    Emitter, Manager, PhysicalPosition, State,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{Mutex, RwLock};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    #[serde(default = "default_taskbar_widget_enabled")]
    taskbar_widget_enabled: bool,
    /// Claude's windows come from its session logs by default. Asking Anthropic's usage
    /// endpoint for the remaining percentage as well presents a stored credential to a
    /// remote service and competes with Claude Code's own reads, so it stays off until it
    /// is turned on deliberately.
    #[serde(default)]
    claude_cross_check_enabled: bool,
    /// Set once the explanation of what that cross-check does has been accepted, so the
    /// consent card is not shown again on every toggle.
    #[serde(default)]
    claude_consent_granted: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            taskbar_widget_enabled: default_taskbar_widget_enabled(),
            claude_cross_check_enabled: false,
            claude_consent_granted: false,
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
        claude_cross_check_enabled: state.claude_cross_check_enabled.load(Ordering::Relaxed),
        claude_consent_granted: state.claude_consent_granted.load(Ordering::Relaxed),
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
    claude_cross_check_enabled: AtomicBool,
    claude_consent_granted: AtomicBool,
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

    /// Whether a provider's optional second quota source may be used this refresh.
    fn cross_check_enabled(&self, provider: ProviderKind) -> bool {
        match provider {
            ProviderKind::Codex => false,
            ProviderKind::Claude => self.claude_cross_check_enabled.load(Ordering::Relaxed),
        }
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

#[tauri::command]
fn set_taskbar_widget_size(
    app: tauri::AppHandle,
    providers: usize,
    windows: usize,
) -> Result<(), String> {
    // A hidden widget still runs its renderer; resizing it must not bring it back.
    if !app.state::<Arc<AppState>>().taskbar_widget_enabled.load(Ordering::Relaxed) {
        return Ok(());
    }
    taskbar::set_widget_size(&app, providers, windows)
}

/// What the renderer needs to show the Claude cross-check opt-in: whether it is on, and
/// whether the explanation still has to be accepted before it can be.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSettings {
    claude_cross_check_enabled: bool,
    claude_consent_granted: bool,
}

#[tauri::command]
fn get_provider_settings(state: State<'_, Arc<AppState>>) -> ProviderSettings {
    ProviderSettings {
        claude_cross_check_enabled: state.claude_cross_check_enabled.load(Ordering::Relaxed),
        claude_consent_granted: state.claude_consent_granted.load(Ordering::Relaxed),
    }
}

/// Turning the cross-check on requires consent, which the renderer collects. Accepting it
/// is what makes the first enable possible; after that the tray toggle is enough.
#[tauri::command]
async fn set_claude_cross_check(
    app: tauri::AppHandle,
    enabled: bool,
    grant_consent: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<ProviderSettings, String> {
    if grant_consent {
        state.claude_consent_granted.store(true, Ordering::Relaxed);
    }
    if enabled && !state.claude_consent_granted.load(Ordering::Relaxed) {
        return Err("The online quota cross-check needs to be confirmed before it can be enabled.".to_string());
    }
    let state = state.inner().clone();
    apply_claude_cross_check(&app, &state, enabled);
    Ok(ProviderSettings {
        claude_cross_check_enabled: state.claude_cross_check_enabled.load(Ordering::Relaxed),
        claude_consent_granted: state.claude_consent_granted.load(Ordering::Relaxed),
    })
}

fn apply_claude_cross_check(app: &tauri::AppHandle, state: &Arc<AppState>, enabled: bool) {
    state.claude_cross_check_enabled.store(enabled, Ordering::Relaxed);
    if let Err(error) = save_settings(state) {
        eprintln!("failed to save application settings: {error}");
    }
    let app = app.clone();
    let state = state.clone();
    // Claude keeps reporting either way; this only changes where its percentages come
    // from, so the next refresh is enough to show the change.
    tauri::async_runtime::spawn(async move {
        refresh::refresh_all(&app, &state).await;
    });
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
                .map_err(|error| error.to_string())?,
        );
    }
    let retention = state.storage.load_retention_diagnostics().await.map_err(|error| error.to_string())?;
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
const QUICK_PANEL_HEIGHT: u32 = 600;

fn quick_panel_size(providers: usize) -> tauri::PhysicalSize<u32> {
    tauri::PhysicalSize::new(
        QUICK_PANEL_COLUMN_WIDTH * providers.clamp(1, 2) as u32,
        QUICK_PANEL_HEIGHT,
    )
}

fn toggle_quick_panel(app: &tauri::AppHandle, click: PhysicalPosition<f64>, tray_rect: tauri::Rect) {
    let Some(panel) = app.get_webview_window("quick-panel") else { return };
    let state = app.state::<Arc<AppState>>();
    let _ = panel.set_size(quick_panel_size(state.enabled_providers().len()));
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

    let size = panel
        .outer_size()
        .unwrap_or_else(|_| quick_panel_size(state.enabled_providers().len()));
    let monitor = app.monitor_from_point(click.x, click.y).ok().flatten();
    let (x, y) = if let Some(monitor) = monitor {
        let origin = monitor.position();
        let bounds = monitor.size();
        let scale_factor = monitor.scale_factor();
        let tray_position = tray_rect.position.to_physical::<f64>(scale_factor);
        let tray_size = tray_rect.size.to_physical::<f64>(scale_factor);
        let anchor = PhysicalPosition::new(
            tray_position.x + tray_size.width / 2.0,
            tray_position.y + tray_size.height / 2.0,
        );
        let left = origin.x as f64;
        let top = origin.y as f64;
        let right = left + bounds.width as f64;
        let bottom = top + bounds.height as f64;
        let panel_width = size.width as f64;
        let panel_height = size.height as f64;
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
        let margin = 12.0;
        let clamp_x = |value: f64| value.clamp(left + margin, right - panel_width - margin);
        let clamp_y = |value: f64| value.clamp(top + margin, bottom - panel_height - margin);
        match nearest {
            "top" => (clamp_x(tray_position.x + tray_size.width - panel_width), tray_position.y + tray_size.height + margin),
            "left" => (tray_position.x + tray_size.width + margin, clamp_y(tray_position.y + tray_size.height - panel_height)),
            "right" => (tray_position.x - panel_width - margin, clamp_y(tray_position.y + tray_size.height - panel_height)),
            _ => (clamp_x(tray_position.x + tray_size.width - panel_width), tray_position.y - panel_height - margin),
        }
    } else {
        (click.x - size.width as f64, click.y - size.height as f64)
    };
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = panel.show();
    let _ = panel.set_focus();
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
    let claude_cross_check_enabled = app
        .state::<Arc<AppState>>()
        .claude_cross_check_enabled
        .load(Ordering::Relaxed);
    let claude_cross_check = CheckMenuItem::with_id(
        app,
        "claude_cross_check",
        "Check Claude quota online",
        true,
        claude_cross_check_enabled,
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
            &claude_cross_check,
            &separator_before_quit,
            &quit,
        ],
    )?;
    let icon = app.default_window_icon().cloned().expect("application icon must be configured");
    let autostart_menu_item = autostart.clone();
    let taskbar_widget_menu_item = taskbar_widget.clone();
    let claude_cross_check_menu_item = claude_cross_check.clone();
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
            "claude_cross_check" => {
                let state = app.state::<Arc<AppState>>().inner().clone();
                let enabled = !state.claude_cross_check_enabled.load(Ordering::Relaxed);
                if enabled && !state.claude_consent_granted.load(Ordering::Relaxed) {
                    // The first enable has to be explained before it takes effect, and
                    // the dashboard is where that explanation lives. Saying so is what
                    // keeps the unchanged tick from reading as a broken menu item.
                    let _ = claude_cross_check_menu_item.set_checked(false);
                    show_main(app);
                    let _ = app.emit_to("main", "claude-consent-requested", ());
                    return;
                }
                apply_claude_cross_check(app, &state, enabled);
                let _ = claude_cross_check_menu_item.set_checked(enabled);
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
                claude_cross_check_enabled: AtomicBool::new(settings.claude_cross_check_enabled),
                claude_consent_granted: AtomicBool::new(settings.claude_consent_granted),
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
            get_provider_settings,
            set_claude_cross_check,
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
