//! The tray icon and its menu, and showing the dashboard they lead to.

use std::sync::Arc;

use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::quick_panel::toggle_quick_panel;
use crate::{AppState, log, refresh};

#[tauri::command]
pub(crate) fn open_dashboard(app: tauri::AppHandle) {
    log::write("dashboard opened from the quick panel");
    if let Some(panel) = app.get_webview_window("quick-panel") {
        let _ = panel.hide();
    }
    show_main(&app);
}

/// Windows refuses a raise request from a process that does not own the foreground, which
/// a tray menu click does not, so a window that is merely behind another one stays there
/// after `set_focus`. Briefly claiming always-on-top is what actually brings it forward.
pub(crate) fn show_main(app: &tauri::AppHandle) {
    log::write("dashboard window shown");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_always_on_top(true);
        let _ = window.set_focus();
        let _ = window.set_always_on_top(false);
    }
}

/// The tray menu carries what has to work when no window is open: showing the dashboard,
/// a manual refresh, and quitting. Every preference lives in the settings dialog instead,
/// so a setting is changed in one place rather than in whichever surface found it first.
pub(crate) fn build_tray(app: &tauri::App) -> tauri::Result<()> {
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
            "show" => {
                log::write("tray menu: show the dashboard");
                show_main(app);
            }
            "refresh" => {
                log::write("tray menu: refresh now");
                let app = app.clone();
                let state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    refresh::refresh_all(&app, &state).await;
                });
            }
            "quit" => {
                log::write("tray menu: quit");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                position,
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_quick_panel(tray.app_handle(), position, rect);
            }
        })
        .build(app)?;
    Ok(())
}
