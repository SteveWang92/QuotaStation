#[cfg(windows)]
use tauri::{Manager, PhysicalPosition, PhysicalSize};

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{
            FindWindowExW, FindWindowW, GetParent, GetWindowLongW, GetWindowRect, MoveWindow, SetParent,
            SetWindowLongW, GWL_EXSTYLE, GWL_STYLE, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_POPUP,
        },
    },
    core::{PCWSTR, w},
};

#[cfg(windows)]
fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect).ok()?; }
    Some(rect)
}

#[cfg(windows)]
pub fn position_widget(app: &tauri::AppHandle) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let taskbar = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
        .map_err(|error| error.to_string())?;
    let taskbar_rect = window_rect(taskbar).ok_or("unable to read taskbar bounds")?;
    let taskbar_width = taskbar_rect.right - taskbar_rect.left;
    let taskbar_height = taskbar_rect.bottom - taskbar_rect.top;
    if taskbar_width <= taskbar_height {
        let _ = widget.hide();
        return Err("vertical taskbars are not supported yet".to_string());
    }

    let tray = unsafe { FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), PCWSTR::null()) }.ok();
    let tray_left = tray.and_then(window_rect).map(|rect| rect.left).unwrap_or(taskbar_rect.right);
    let requested_width = widget.outer_size().map_err(|error| error.to_string())?.width as i32;
    let width = requested_width.min((tray_left - taskbar_rect.left - 16).max(140));
    let height = (taskbar_height - 8).clamp(30, 44);
    let x = (tray_left - taskbar_rect.left - width - 8).max(8);
    let y = ((taskbar_height - height) / 2).max(0);
    let hwnd = widget.hwnd().map_err(|error| error.to_string())?;
    unsafe {
        if GetParent(hwnd).ok() != Some(taskbar) {
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_TOOLWINDOW.0 as i32 | WS_EX_NOACTIVATE.0 as i32);
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let child_style = (style & !WS_POPUP.0) | WS_CHILD.0 | WS_CLIPSIBLINGS.0;
            SetWindowLongW(hwnd, GWL_STYLE, child_style as i32);
            SetParent(hwnd, Some(taskbar)).map_err(|error| error.to_string())?;
        }
        let current = window_rect(hwnd).ok_or("unable to read taskbar widget bounds")?;
        let expected_left = taskbar_rect.left + x;
        let expected_top = taskbar_rect.top + y;
        if current.left != expected_left
            || current.top != expected_top
            || current.right - current.left != width
            || current.bottom - current.top != height
        {
            MoveWindow(hwnd, x, y, width, height, true).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn set_widget_columns(app: &tauri::AppHandle, columns: usize) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let width = if columns <= 1 { 148 } else { 270 };
    widget.set_size(PhysicalSize::new(width, 40)).map_err(|error| error.to_string())?;
    position_widget(app)
}

#[cfg(windows)]
pub fn position_widget_fallback(app: &tauri::AppHandle) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let monitor = app.primary_monitor().map_err(|error| error.to_string())?.ok_or("primary monitor missing")?;
    let origin = monitor.position();
    let size = monitor.size();
    let width = 270_u32;
    let height = 40_u32;
    widget.set_size(PhysicalSize::new(width, height)).map_err(|error| error.to_string())?;
    widget
        .set_position(PhysicalPosition::new(
            origin.x + size.width as i32 - width as i32 - 12,
            origin.y + size.height as i32 - height as i32 - 60,
        ))
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(not(windows))]
pub fn position_widget(_app: &tauri::AppHandle) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(not(windows))]
pub fn position_widget_fallback(_app: &tauri::AppHandle) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(not(windows))]
pub fn set_widget_columns(_app: &tauri::AppHandle, _columns: usize) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}
