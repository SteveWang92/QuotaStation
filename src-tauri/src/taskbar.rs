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
    // The widget is a child of the taskbar, so anything taller than the taskbar is simply
    // cropped by it — and the crop takes the bottom row of a two-window reading with it.
    // The height therefore follows the taskbar down instead of holding a minimum it may not
    // have room for; the interface is laid out to survive the short end of this range.
    let height = (taskbar_height - 6).clamp(20, 44);
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

/// The widget keeps one size whatever it is showing. Its width used to follow the provider
/// count and the number of windows each was reporting, which meant it grew sideways as
/// providers finished loading and shrank again when one stopped answering: the neighbouring
/// taskbar icons moved every time. A fixed slot fits the widest case — two providers, two
/// windows each — and the content is right-aligned inside it, so a narrower reading simply
/// leaves transparent space where the taskbar shows through. `place_widget` still narrows
/// the window when the taskbar is short of room beside the notification area.
pub const WIDGET_WIDTH: u32 = 460;
pub const WIDGET_HEIGHT: u32 = 40;

#[cfg(windows)]
pub fn set_widget_size(app: &tauri::AppHandle) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    widget
        .set_size(PhysicalSize::new(WIDGET_WIDTH, WIDGET_HEIGHT))
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
pub fn set_widget_size(_app: &tauri::AppHandle) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(test)]
mod tests {
    use super::{WIDGET_HEIGHT, WIDGET_WIDTH};

    #[test]
    fn the_widget_reserves_one_slot_for_every_reading_it_can_show() {
        // Two providers showing two windows each is the widest the interface goes, and the
        // taskbar clamps anything taller than this.
        assert!(WIDGET_WIDTH >= 2 * 200, "two provider columns must fit");
        assert!((30..=44).contains(&WIDGET_HEIGHT), "the taskbar clamps its own height");
    }
}
