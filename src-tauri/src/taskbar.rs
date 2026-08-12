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

/// Docks the widget inside the taskbar, falling back to a floating window whenever the
/// taskbar cannot host it. The widget stays visible either way; a failed dock must never
/// leave the user with a status surface that nothing brings back.
#[cfg(windows)]
pub fn place_widget(app: &tauri::AppHandle) -> Result<(), String> {
    match dock_widget(app) {
        Ok(()) => Ok(()),
        Err(reason) => match float_widget(app) {
            Ok(()) => Err(format!("{reason}; showing the status as a floating window instead")),
            Err(error) => Err(format!("{reason}; floating fallback failed: {error}")),
        },
    }
}

#[cfg(windows)]
fn dock_widget(app: &tauri::AppHandle) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let taskbar = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
        .map_err(|error| error.to_string())?;
    let taskbar_rect = window_rect(taskbar).ok_or("unable to read taskbar bounds")?;
    let taskbar_width = taskbar_rect.right - taskbar_rect.left;
    let taskbar_height = taskbar_rect.bottom - taskbar_rect.top;
    if taskbar_width <= taskbar_height {
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

/// The widget grows sideways only: the taskbar clamps its height, so a second provider
/// has to become a second column. A provider showing one window needs less room than one
/// showing both, and `place_widget` narrows the result again when the taskbar is short of
/// space beside the notification area.
pub fn widget_width(providers: usize, windows: usize) -> u32 {
    let column = if windows <= 1 { 148 } else { 270 };
    column * providers.clamp(1, 3) as u32
}

#[cfg(windows)]
pub fn set_widget_size(app: &tauri::AppHandle, providers: usize, windows: usize) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    widget
        .set_size(PhysicalSize::new(widget_width(providers, windows), 40))
        .map_err(|error| error.to_string())?;
    place_widget(app)
}

/// Parks the widget above the taskbar as an ordinary window. Every step is skipped when
/// it already holds, so the repositioning loop does not fight the window every tick.
#[cfg(windows)]
fn float_widget(app: &tauri::AppHandle) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let hwnd = widget.hwnd().map_err(|error| error.to_string())?;
    let mut detached = false;
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_CHILD.0 != 0 {
            SetWindowLongW(hwnd, GWL_STYLE, ((style & !WS_CHILD.0) | WS_POPUP.0) as i32);
            SetParent(hwnd, None).map_err(|error| error.to_string())?;
            detached = true;
        }
    }
    if detached {
        // Parenting the widget to the taskbar dropped its topmost placement.
        widget.set_always_on_top(true).map_err(|error| error.to_string())?;
    }

    let monitor = app.primary_monitor().map_err(|error| error.to_string())?.ok_or("primary monitor missing")?;
    let origin = monitor.position();
    let bounds = monitor.size();
    let size = widget.outer_size().map_err(|error| error.to_string())?;
    let position = PhysicalPosition::new(
        origin.x + bounds.width as i32 - size.width as i32 - 12,
        origin.y + bounds.height as i32 - size.height as i32 - 60,
    );
    if widget.outer_position().map_err(|error| error.to_string())? != position {
        widget.set_position(position).map_err(|error| error.to_string())?;
    }
    if !widget.is_visible().unwrap_or(true) {
        widget.show().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn place_widget(_app: &tauri::AppHandle) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(not(windows))]
pub fn set_widget_size(_app: &tauri::AppHandle, _providers: usize, _windows: usize) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(test)]
mod tests {
    use super::widget_width;

    #[test]
    fn a_second_provider_widens_the_widget_instead_of_stacking_it() {
        assert_eq!(widget_width(1, 2), 270);
        assert_eq!(widget_width(2, 2), 540);
        // A provider reporting a single window needs a narrower column.
        assert_eq!(widget_width(1, 1), 148);
        assert_eq!(widget_width(2, 1), 296);
        // A provider count of zero still has to produce a usable window.
        assert_eq!(widget_width(0, 2), 270);
    }
}
