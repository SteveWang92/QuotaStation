mod domain;
mod providers;
mod refresh;
mod storage;

use std::{sync::Arc, time::Duration};

use domain::ProviderSnapshot;
use storage::Storage;
use tauri::{
    Manager, State,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{Mutex, RwLock};

pub struct AppState {
    storage: Storage,
    snapshot: RwLock<ProviderSnapshot>,
    refresh_lock: Mutex<()>,
}

#[tauri::command]
async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<ProviderSnapshot, String> {
    Ok(state.snapshot.read().await.clone())
}

#[tauri::command]
async fn refresh_now(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<ProviderSnapshot, String> {
    Ok(refresh::refresh_all(&app, state.inner()).await)
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show QuotaStation", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &refresh, &quit])?;
    let icon = app.default_window_icon().cloned().expect("application icon must be configured");
    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
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
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let database_path = app.path().app_data_dir()?.join("quotastation.db");
            let storage = tauri::async_runtime::block_on(Storage::open(&database_path))
                .map_err(|error| error.to_string())?;
            let snapshot = tauri::async_runtime::block_on(storage.load_snapshot())
                .unwrap_or_default();
            let state = Arc::new(AppState {
                storage,
                snapshot: RwLock::new(snapshot),
                refresh_lock: Mutex::new(()),
            });
            app.manage(state.clone());
            build_tray(app)?;
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                refresh::refresh_all(&app_handle, &state).await;
                let mut interval = tokio::time::interval(Duration::from_secs(300));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    refresh::refresh_all(&app_handle, &state).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_snapshot, refresh_now])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running QuotaStation");
}
