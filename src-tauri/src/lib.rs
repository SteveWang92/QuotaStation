mod domain;
mod providers;
mod refresh;
mod session_watcher;
mod storage;

use std::{path::PathBuf, sync::Arc, time::Duration};

use domain::{DiagnosticsSnapshot, ProviderSnapshot, UsageRangeSnapshot, WatcherDiagnostics};
use storage::Storage;
use tauri::{
    Manager, State,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{Mutex, RwLock};

#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

pub struct AppState {
    storage: Storage,
    snapshot: RwLock<ProviderSnapshot>,
    live_refresh_lock: Mutex<()>,
    history_refresh_lock: Mutex<()>,
    watcher_diagnostics: RwLock<WatcherDiagnostics>,
}

#[tauri::command]
async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<ProviderSnapshot, String> {
    Ok(state.snapshot.read().await.clone())
}

#[tauri::command]
async fn get_usage_range(
    start_date: String,
    end_date: String,
    state: State<'_, Arc<AppState>>,
) -> Result<UsageRangeSnapshot, String> {
    state.storage.load_usage_range(&start_date, &end_date).await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn refresh_now(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<ProviderSnapshot, String> {
    Ok(refresh::refresh_all(&app, state.inner()).await)
}

#[tauri::command]
async fn get_diagnostics(state: State<'_, Arc<AppState>>) -> Result<DiagnosticsSnapshot, String> {
    let acquisitions = state.storage.load_acquisition_diagnostics().await.map_err(|error| error.to_string())?;
    let retention = state.storage.load_retention_diagnostics().await.map_err(|error| error.to_string())?;
    Ok(DiagnosticsSnapshot {
        watcher: state.watcher_diagnostics.read().await.clone(),
        acquisitions,
        retention,
        parser_revision: domain::CCUSAGE_REVISION.to_string(),
        pricing_catalog_revision: domain::PRICING_CATALOG_REVISION.to_string(),
    })
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
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
            &separator_before_quit,
            &quit,
        ],
    )?;
    let icon = app.default_window_icon().cloned().expect("application icon must be configured");
    let autostart_menu_item = autostart.clone();
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
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
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
            let database_path = app.path().app_data_dir()?.join("quotastation.db");
            let storage = tauri::async_runtime::block_on(Storage::open(&database_path))
                .map_err(|error| error.to_string())?;
            if let Err(error) = tauri::async_runtime::block_on(storage.run_retention_if_due()) {
                eprintln!("normalized data retention failed: {error:#}");
            }
            let snapshot = tauri::async_runtime::block_on(storage.load_snapshot())
                .unwrap_or_default();
            let state = Arc::new(AppState {
                storage,
                snapshot: RwLock::new(snapshot),
                live_refresh_lock: Mutex::new(()),
                history_refresh_lock: Mutex::new(()),
                watcher_diagnostics: RwLock::new(WatcherDiagnostics::default()),
            });
            app.manage(state.clone());
            build_tray(app)?;
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
            let app_handle = app.handle().clone();
            let live_state = app.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(300));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    refresh::refresh_live(&app_handle, &live_state).await;
                }
            });
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
        .invoke_handler(tauri::generate_handler![get_snapshot, get_usage_range, refresh_now, get_diagnostics])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running QuotaStation");
}
