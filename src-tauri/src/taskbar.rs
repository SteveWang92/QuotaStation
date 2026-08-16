#[cfg(windows)]
use tauri::{Manager, PhysicalPosition, PhysicalSize};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                CallNextHookEx, FindWindowExW, FindWindowW, GA_ROOT, GetAncestor,
                GetForegroundWindow, GetParent, GetWindowLongW, GetWindowRect,
                GetWindowThreadProcessId, HC_ACTION, IsWindowVisible, MSLLHOOKSTRUCT, MoveWindow,
                SetForegroundWindow, SetParent, WindowFromPoint,
                SetWindowLongW, SetWindowsHookExW, GWL_EXSTYLE, GWL_STYLE, WH_MOUSE_LL,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_POPUP,
            },
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
    if let Some(hwnd) = app.get_webview_window("taskbar-widget").and_then(|widget| widget.hwnd().ok()) {
        WATCHED_WIDGET.store(hwnd.0 as isize, Ordering::Relaxed);
    }
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
    let available_width = (tray_left - taskbar_rect.left - 16).max(0);
    if available_width < requested_width {
        return Err(format!(
            "taskbar has {available_width}px available but the status layout requires {requested_width}px"
        ));
    }
    let width = requested_width;
    // Use the taskbar's actual physical height. A 44px ceiling left only 22 CSS pixels at
    // 200% scaling and cropped the second quota row even when Explorer had ample space.
    let height = docked_height(taskbar_height);
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

/// Where the widget is on screen, for anchoring the panel a click on it opens.
///
/// Read from the window handle rather than from Tauri: once the widget is docked it is a
/// child of the taskbar, and its Tauri position is then relative to that parent while
/// `GetWindowRect` stays in screen coordinates whether it is docked or floating.
#[cfg(windows)]
pub fn widget_screen_rect(
    app: &tauri::AppHandle,
) -> Result<(PhysicalPosition<f64>, PhysicalSize<f64>), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let hwnd = widget.hwnd().map_err(|error| error.to_string())?;
    let rect = window_rect(hwnd).ok_or("unable to read taskbar widget bounds")?;
    Ok((
        PhysicalPosition::new(f64::from(rect.left), f64::from(rect.top)),
        PhysicalSize::new(
            f64::from(rect.right - rect.left),
            f64::from(rect.bottom - rect.top),
        ),
    ))
}

/// The docked widget, for the click watch below, or zero while nothing is docked.
#[cfg(windows)]
static WATCHED_WIDGET: AtomicIsize = AtomicIsize::new(0);

/// Whether the press half of a left click was taken for the widget. A global hook must
/// consume a complete pair: swallowing only the release can leave another application in
/// mouse capture, while swallowing only the press hands it a release it never asked for.
#[cfg(windows)]
static WIDGET_CLICK_CAPTURED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WidgetClickAction {
    Pass,
    Swallow,
    Open,
}

#[cfg(windows)]
fn widget_click_transition(
    captured: bool,
    message: u32,
    pointer_over_widget: bool,
) -> (bool, WidgetClickAction) {
    match message {
        WM_LBUTTONDOWN if pointer_over_widget => (true, WidgetClickAction::Swallow),
        WM_LBUTTONUP if captured && pointer_over_widget => (false, WidgetClickAction::Open),
        WM_LBUTTONUP if captured => (false, WidgetClickAction::Swallow),
        _ => (captured, WidgetClickAction::Pass),
    }
}

/// Opens the quick panel when the left button is released over the docked widget.
///
/// **Why a system hook rather than a click handler in the widget.** Explorer takes every
/// mouse message over the taskbar for itself: with the widget docked, its webview receives
/// no click and no pointer movement at all — measured, not assumed. A `WM_LBUTTONUP`
/// handler on the window would see nothing either, because the message never arrives. A
/// low-level mouse hook sits ahead of that routing, and it is the only place the click is
/// still ours to read.
///
/// It reads nothing but the pointer and the left button, and only acts on a release inside
/// the widget's own rectangle. That release is consumed so Explorer does not answer the same
/// click by giving the taskbar the foreground, which is what left the panel opening behind
/// it and closing again on the focus it never got. Every other event is passed straight on.
#[cfg(windows)]
unsafe extern "system" fn on_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(wparam.0 as u32, WM_LBUTTONDOWN | WM_LBUTTONUP) {
        let point = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
        let captured = WIDGET_CLICK_CAPTURED.load(Ordering::Relaxed);
        let (next, action) = widget_click_transition(captured, wparam.0 as u32, over_widget(point));
        WIDGET_CLICK_CAPTURED.store(next, Ordering::Relaxed);
        match action {
            WidgetClickAction::Open => {
                crate::open_quick_panel_from_taskbar();
                return LRESULT(1);
            }
            WidgetClickAction::Swallow => return LRESULT(1),
            WidgetClickAction::Pass => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(windows)]
fn over_widget(point: POINT) -> bool {
    let hwnd = HWND(WATCHED_WIDGET.load(Ordering::Relaxed) as *mut _);
    if hwnd.is_invalid() || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }
    // A visible-style window can still sit behind a fullscreen or always-on-top window.
    // The hook is global, so a rectangle check alone would steal that covering window's
    // click. WindowFromPoint identifies the root that will actually receive it.
    let hit = unsafe { WindowFromPoint(point) };
    if hit.is_invalid()
        || unsafe { GetAncestor(hit, GA_ROOT) } != unsafe { GetAncestor(hwnd, GA_ROOT) }
    {
        return false;
    }
    window_rect(hwnd).is_some_and(|rect| {
        (rect.left..rect.right).contains(&point.x) && (rect.top..rect.bottom).contains(&point.y)
    })
}

/// Installs the click watch. Must run on the thread with the message loop — a low-level
/// hook is called on the thread that set it, and a thread that never pumps messages is one
/// the system eventually drops the hook from.
#[cfg(windows)]
pub fn watch_widget_clicks() {
    static INSTALLED: AtomicIsize = AtomicIsize::new(0);
    if INSTALLED.swap(1, Ordering::SeqCst) == 1 {
        return;
    }
    if let Err(error) = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), None, 0) } {
        INSTALLED.store(0, Ordering::SeqCst);
        crate::log::write(format!("taskbar status click watch unavailable: {error}"));
    }
}

#[cfg(not(windows))]
pub fn watch_widget_clicks() {}

/// Hands a window the foreground when the click that asked for it landed on somebody else's
/// window.
///
/// Windows only grants the foreground to the process that owns the click. A click on the
/// docked widget belongs to Explorer — the widget is a child of the taskbar — so the panel
/// this opens is shown, refused the foreground, and told immediately that it lost focus,
/// which its own dismissal then acts on. Borrowing the input queue of whichever thread does
/// hold the foreground is the documented way to be allowed to take it.
#[cfg(windows)]
pub fn raise_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    // On the thread that owns the window: the input queues being attached are the ones the
    // calling thread has, and a command thread has none of the window's.
    let handle = app.clone();
    let label = label.to_string();
    app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(&label) else { return };
        let Ok(hwnd) = window.hwnd() else { return };
        unsafe {
            let holder = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let ours = GetCurrentThreadId();
            let borrowed =
                holder != 0 && holder != ours && AttachThreadInput(holder, ours, true).as_bool();
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
            if borrowed {
                let _ = AttachThreadInput(holder, ours, false);
            }
        }
    })
    .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
pub fn raise_window(_app: &tauri::AppHandle, _label: &str) -> Result<(), String> {
    Ok(())
}

/// Two provider slots are always reserved, so ordinary loading and provider failures do not
/// move neighbouring taskbar icons. A future third provider grows the contract by one whole
/// slot rather than overflowing a width designed for Codex and Claude only. If Explorer cannot
/// provide the complete contract width, docking fails and the full layout floats instead of
/// being squeezed until its leftmost provider is cropped.
///
/// The width is the width the layout is drawn at, so it is scaled by the monitor's factor
/// before the window is sized: asking for 460 device pixels on a 125% display left the
/// renderer 368 CSS pixels to lay out 441 in, and the columns that overflowed were cropped
/// off the left edge — the first provider lost its name. The height stays in device pixels
/// because the taskbar, not the layout, decides it.
const WIDGET_BASE_WIDTH: u32 = 40;
const PROVIDER_SLOT_WIDTH: u32 = 210;
const MIN_PROVIDER_SLOTS: u32 = 2;
const MAX_PROVIDER_SLOTS: u32 = 8;
pub const WIDGET_HEIGHT: u32 = 40;

pub fn widget_width(provider_count: u32) -> u32 {
    let slots = provider_count.clamp(MIN_PROVIDER_SLOTS, MAX_PROVIDER_SLOTS);
    WIDGET_BASE_WIDTH.saturating_add(slots.saturating_mul(PROVIDER_SLOT_WIDTH))
}

fn docked_height(taskbar_height: i32) -> i32 {
    (taskbar_height - 6).max(20)
}

#[cfg(windows)]
pub fn set_widget_size(app: &tauri::AppHandle, provider_count: u32) -> Result<(), String> {
    let widget = app.get_webview_window("taskbar-widget").ok_or("taskbar widget window missing")?;
    let scale = widget.scale_factor().unwrap_or(1.0).max(1.0);
    widget
        .set_size(PhysicalSize::new(
            (f64::from(widget_width(provider_count)) * scale).round() as u32,
            WIDGET_HEIGHT,
        ))
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
pub fn set_widget_size(_app: &tauri::AppHandle, _provider_count: u32) -> Result<(), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(not(windows))]
pub fn widget_screen_rect(
    _app: &tauri::AppHandle,
) -> Result<(tauri::PhysicalPosition<f64>, tauri::PhysicalSize<f64>), String> {
    Err("taskbar status is supported on Windows only".to_string())
}

#[cfg(test)]
mod tests {
    use super::{WIDGET_HEIGHT, docked_height, widget_width};

    #[cfg(windows)]
    use super::{WidgetClickAction, widget_click_transition};

    #[cfg(windows)]
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};

    #[test]
    fn the_widget_reserves_one_slot_for_every_reading_it_can_show() {
        // Two providers showing two windows each is the widest the interface goes, and the
        // taskbar clamps anything taller than this.
        assert!(widget_width(2) >= 2 * 200, "two provider columns must fit");
        assert!(WIDGET_HEIGHT >= 30, "the initial window must fit both rows before docking");
    }

    #[test]
    fn provider_capacity_grows_only_after_the_reserved_two_slots() {
        let reserved_width = widget_width(2);
        assert_eq!(widget_width(0), reserved_width);
        assert_eq!(widget_width(1), reserved_width);
        assert!(widget_width(3) > reserved_width);
        assert_eq!(widget_width(u32::MAX), widget_width(8), "renderer input is bounded");
    }

    #[test]
    fn docked_height_follows_high_dpi_taskbars_instead_of_clipping_them() {
        assert_eq!(docked_height(40), 34);
        assert_eq!(docked_height(80), 74);
        assert_eq!(docked_height(18), 20, "a malformed tiny taskbar still gets a legal window");
    }

    #[cfg(windows)]
    #[test]
    fn a_widget_press_consumes_its_matching_release_even_after_a_drag_out() {
        let (captured, down) = widget_click_transition(false, WM_LBUTTONDOWN, true);
        assert!(captured);
        assert_eq!(down, WidgetClickAction::Swallow);

        let (captured, up) = widget_click_transition(captured, WM_LBUTTONUP, false);
        assert!(!captured);
        assert_eq!(up, WidgetClickAction::Swallow);
    }

    #[cfg(windows)]
    #[test]
    fn a_release_dragged_in_from_another_window_is_left_alone() {
        let (captured, action) = widget_click_transition(false, WM_LBUTTONUP, true);
        assert!(!captured);
        assert_eq!(action, WidgetClickAction::Pass);
    }

    #[cfg(windows)]
    #[test]
    fn a_complete_click_on_the_widget_opens_the_panel() {
        let (captured, down) = widget_click_transition(false, WM_LBUTTONDOWN, true);
        assert_eq!(down, WidgetClickAction::Swallow);
        let (captured, up) = widget_click_transition(captured, WM_LBUTTONUP, true);
        assert!(!captured);
        assert_eq!(up, WidgetClickAction::Open);
    }
}
