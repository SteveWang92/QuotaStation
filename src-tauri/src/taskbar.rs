#[cfg(windows)]
use tauri::{Manager, PhysicalPosition, PhysicalSize};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use serde::Serialize;

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, TRUE, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW,
            MonitorFromWindow,
        },
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                CallNextHookEx, EnumChildWindows, FindWindowExW, FindWindowW, GA_ROOT, GWL_EXSTYLE,
                GWL_STYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetParent,
                GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, HC_ACTION, IsWindow,
                IsWindowVisible, MONITORINFOF_PRIMARY, MSLLHOOKSTRUCT, MoveWindow,
                SetForegroundWindow, SetParent, SetWindowLongW, SetWindowsHookExW, WH_MOUSE_LL,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_POPUP, WindowFromPoint,
            },
        },
    },
    core::{BOOL, PCWSTR, w},
};

#[cfg(windows)]
fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut rect).ok()?;
    }
    Some(rect)
}

/// A display whose taskbar can host the status widget, as the settings dialog offers it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskbarDisplay {
    /// The Windows device name, `\\.\DISPLAY1`. This is what the choice is recorded as: it
    /// survives a restart, unlike a window handle or an index into an enumeration order.
    pub id: String,
    pub label: String,
    pub primary: bool,
}

/// One of Explorer's taskbars, and the display it sits on.
#[cfg(windows)]
struct Taskbar {
    hwnd: HWND,
    display: String,
    primary: bool,
    /// The display's whole rectangle, for the floating fallback.
    monitor: RECT,
    monitor_handle: HMONITOR,
}

/// Every taskbar Explorer is showing: the primary one first, then one per additional
/// display. A secondary taskbar is a `Shell_SecondaryTrayWnd` of its own rather than a
/// child of the primary, so both classes have to be walked.
#[cfg(windows)]
fn taskbars() -> Vec<Taskbar> {
    let mut found = Vec::new();
    if let Ok(primary) = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) } {
        found.extend(describe_taskbar(primary));
    }
    let mut previous: Option<HWND> = None;
    while let Ok(next) =
        unsafe { FindWindowExW(None, previous, w!("Shell_SecondaryTrayWnd"), PCWSTR::null()) }
    {
        found.extend(describe_taskbar(next));
        previous = Some(next);
    }
    found
}

#[cfg(windows)]
fn describe_taskbar(hwnd: HWND) -> Option<Taskbar> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast::<MONITORINFO>()) }
        .as_bool()
    {
        return None;
    }
    let device = String::from_utf16_lossy(&info.szDevice);
    Some(Taskbar {
        hwnd,
        display: device.trim_end_matches('\0').to_string(),
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        monitor: info.monitorInfo.rcMonitor,
        monitor_handle: monitor,
    })
}

/// The taskbar the widget belongs on: the chosen display's, or the primary one whenever no
/// display was chosen and whenever the chosen one is no longer attached. A monitor that
/// comes and goes must not leave the status with nowhere to be.
#[cfg(windows)]
fn chosen_taskbar(app: &tauri::AppHandle) -> Option<Taskbar> {
    let preferred = crate::preferred_taskbar_display(app);
    let mut taskbars = taskbars();
    if let Some(preferred) = preferred.as_deref()
        && let Some(index) = taskbars.iter().position(|taskbar| taskbar.display == preferred)
    {
        return Some(taskbars.swap_remove(index));
    }
    if let Some(index) = taskbars.iter().position(|taskbar| taskbar.primary) {
        return Some(taskbars.swap_remove(index));
    }
    taskbars.into_iter().next()
}

/// The displays the settings dialog can offer, named the way a person picks between them.
#[cfg(windows)]
pub fn taskbar_displays() -> Vec<TaskbarDisplay> {
    taskbars()
        .into_iter()
        .map(|taskbar| TaskbarDisplay {
            label: display_label(
                &taskbar.display,
                taskbar.primary,
                taskbar.monitor.right - taskbar.monitor.left,
                taskbar.monitor.bottom - taskbar.monitor.top,
            ),
            id: taskbar.display,
            primary: taskbar.primary,
        })
        .collect()
}

#[cfg(not(windows))]
pub fn taskbar_displays() -> Vec<TaskbarDisplay> {
    Vec::new()
}

/// `\\.\DISPLAY2` reads as nothing at all in a menu, so the number is paired with the
/// resolution — which is how a person tells two attached screens apart.
fn display_label(device: &str, primary: bool, width: i32, height: i32) -> String {
    let name = match device.split_once("DISPLAY") {
        Some((_, number)) if !number.is_empty() => format!("Display {number}"),
        _ => device.to_string(),
    };
    let primary = if primary { " (primary)" } else { "" };
    format!("{name}{primary} — {width} × {height}")
}

/// The widget window's label, which gains a suffix every time the window has to be built
/// again.
///
/// A label is claimed for the life of the process: Tauri never released `taskbar-widget`
/// after Explorer destroyed the window underneath it — measured, and `destroy()` on the
/// record it kept does not free it either, because that path waits for a window that is
/// already gone. So the replacement takes the next name rather than the old one. Everything
/// that addresses the widget asks for the current label instead of spelling it out.
static WIDGET_GENERATION: AtomicU32 = AtomicU32::new(0);

pub fn widget_label() -> String {
    match WIDGET_GENERATION.load(Ordering::Relaxed) {
        0 => "taskbar-widget".to_string(),
        generation => format!("taskbar-widget-{generation}"),
    }
}

/// The widget's window handle, or `None` once that window no longer exists.
///
/// Explorer owns the docked widget: it is a child of the taskbar, so restarting Explorer
/// destroys the taskbar and takes the widget with it. Asking Tauri for the size or position
/// of a window that is already gone panics inside tao — measured, on the thread running the
/// event loop, which ends the whole application rather than the widget. Nothing here calls
/// into the window until its handle is known to still be alive.
#[cfg(windows)]
fn live_widget(app: &tauri::AppHandle) -> Option<HWND> {
    let hwnd = app.get_webview_window(&widget_label())?.hwnd().ok()?;
    unsafe { IsWindow(Some(hwnd)) }.as_bool().then_some(hwnd)
}

/// Builds the widget's window again after Explorer took it, from the same configuration it
/// is created with at startup. Placement runs on a loop, so the next tick docks it.
#[cfg(windows)]
fn rebuild_widget(app: &tauri::AppHandle) -> Result<(), String> {
    // Building a webview window takes longer than the two seconds between placement ticks,
    // and every tick until it appears would ask for another one: three status windows were
    // created from one Explorer restart before this guard existed.
    if REBUILD_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let Some(mut config) =
        app.config().app.windows.iter().find(|window| window.label == "taskbar-widget").cloned()
    else {
        REBUILD_IN_FLIGHT.store(false, Ordering::SeqCst);
        return Err("the taskbar status window is not configured".to_string());
    };
    // The generation only moves once the window exists, so a failed build does not strand
    // the label on a window that was never created.
    let generation = WIDGET_GENERATION.load(Ordering::SeqCst) + 1;
    config.label = format!("taskbar-widget-{generation}");
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        // A window is created on the thread that owns the event loop, and the placement
        // loop this is called from is not it. Whether one is still needed is decided here
        // rather than there: several callers reach placement — the loop, the renderer
        // reporting its width, the setting being switched on — and by the time this runs
        // one of the others may already have rebuilt it.
        if live_widget(&handle).is_some() {
            REBUILD_IN_FLIGHT.store(false, Ordering::SeqCst);
            return;
        }
        match tauri::WebviewWindowBuilder::from_config(&handle, &config)
            .and_then(|builder| builder.build())
        {
            Ok(widget) => {
                WIDGET_GENERATION.store(generation, Ordering::SeqCst);
                let _ = widget.show();
                crate::log::write("taskbar status rebuilt after Explorer replaced the taskbar");
            }
            Err(error) => report_rebuild_failure(&error.to_string()),
        }
        REBUILD_IN_FLIGHT.store(false, Ordering::SeqCst);
    });
    if queued.is_err() {
        REBUILD_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
    queued.map_err(|error| error.to_string())
}

#[cfg(windows)]
static REBUILD_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Placement retries every couple of seconds, so the same failure would otherwise be
/// written to the log thirty times a minute and bury everything else in it.
#[cfg(windows)]
fn report_rebuild_failure(message: &str) {
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return };
    if last.as_deref() == Some(message) {
        return;
    }
    crate::log::write(format!("taskbar status could not be rebuilt: {message}"));
    *last = Some(message.to_string());
}

/// Whether the widget's window is there to be shown or hidden.
#[cfg(windows)]
pub fn widget_is_live(app: &tauri::AppHandle) -> bool {
    live_widget(app).is_some()
}

#[cfg(not(windows))]
pub fn widget_is_live(_app: &tauri::AppHandle) -> bool {
    false
}

/// Docks the widget inside the taskbar, falling back to a floating window whenever the
/// taskbar cannot host it. The widget stays visible either way; a failed dock must never
/// leave the user with a status surface that nothing brings back.
#[cfg(windows)]
pub fn place_widget(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(hwnd) = live_widget(app) else {
        WATCHED_WIDGET.store(0, Ordering::Relaxed);
        return rebuild_widget(app);
    };
    WATCHED_WIDGET.store(hwnd.0 as isize, Ordering::Relaxed);
    let taskbar = chosen_taskbar(app);
    match taskbar
        .as_ref()
        .ok_or_else(|| "no taskbar found".to_string())
        .and_then(|taskbar| dock_widget(app, taskbar))
    {
        Ok(()) => Ok(()),
        Err(reason) => match float_widget(app, taskbar.as_ref()) {
            Ok(()) => Err(format!("{reason}; showing the status as a floating window instead")),
            Err(error) => Err(format!("{reason}; floating fallback failed: {error}")),
        },
    }
}

#[cfg(windows)]
fn dock_widget(app: &tauri::AppHandle, taskbar: &Taskbar) -> Result<(), String> {
    let widget = app.get_webview_window(&widget_label()).ok_or("taskbar widget window missing")?;
    let taskbar_rect = window_rect(taskbar.hwnd).ok_or("unable to read taskbar bounds")?;
    let taskbar_width = taskbar_rect.right - taskbar_rect.left;
    let taskbar_height = taskbar_rect.bottom - taskbar_rect.top;
    if taskbar_width <= taskbar_height {
        return Err("vertical taskbars are not supported yet".to_string());
    }

    // The layout is drawn in CSS pixels, so its width follows the display the widget will
    // actually sit on — not the one the window happened to be created on. Two taskbars at
    // different scalings otherwise crop whichever of them is not the primary.
    let dpi = taskbar_dpi(taskbar);
    let width = scaled(widget_width(provider_slots()), dpi);
    let trailing = trailing_edge(taskbar, taskbar_rect, dpi);
    let leading = leading_edge(taskbar.hwnd, taskbar_rect);
    let available_width = (trailing - leading - scaled(TASKBAR_MARGIN, dpi)).max(0);
    if available_width < width {
        return Err(format!(
            "taskbar has {available_width}px available but the status layout requires {width}px"
        ));
    }
    // Use the taskbar's actual physical height. A 44px ceiling left only 22 CSS pixels at
    // 200% scaling and cropped the second quota row even when Explorer had ample space.
    let height = docked_height(taskbar_height);
    let gap = scaled(TASKBAR_MARGIN / 2, dpi);
    let x = (trailing - taskbar_rect.left - width - gap).max(gap);
    let y = ((taskbar_height - height) / 2).max(0);
    let hwnd = widget.hwnd().map_err(|error| error.to_string())?;
    unsafe {
        if GetParent(hwnd).ok() != Some(taskbar.hwnd) {
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            SetWindowLongW(
                hwnd,
                GWL_EXSTYLE,
                ex_style | WS_EX_TOOLWINDOW.0 as i32 | WS_EX_NOACTIVATE.0 as i32,
            );
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let child_style = (style & !WS_POPUP.0) | WS_CHILD.0 | WS_CLIPSIBLINGS.0;
            SetWindowLongW(hwnd, GWL_STYLE, child_style as i32);
            SetParent(hwnd, Some(taskbar.hwnd)).map_err(|error| error.to_string())?;
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

/// Where the widget's right edge goes: immediately before whatever the taskbar keeps at its
/// trailing end.
///
/// The primary taskbar's notification area is a window of its own, so its left edge is read
/// directly. A secondary taskbar has no `TrayNotifyWnd` — measured: its only children are
/// `Start`, `WorkerW`/`MSTaskListWClass` and the XAML content bridge, and its clock is drawn
/// inside that bridge with no window to measure. That end is reserved by width instead,
/// generously enough for a two-line date and time.
#[cfg(windows)]
fn trailing_edge(taskbar: &Taskbar, rect: RECT, dpi: u32) -> i32 {
    unsafe { FindWindowExW(Some(taskbar.hwnd), None, w!("TrayNotifyWnd"), PCWSTR::null()) }
        .ok()
        .and_then(window_rect)
        .map(|tray| tray.left)
        .unwrap_or_else(|| rect.right - scaled(SECONDARY_CLOCK_RESERVE, dpi))
}

/// The display's effective scaling.
///
/// Read from the monitor rather than from the taskbar window: `GetDpiForWindow` answers in
/// terms of the *calling* process's DPI awareness, and it reported a flat 96 for a taskbar
/// on a 125% display — which sized the layout at 460 device pixels where it needed 575 and
/// cropped its leading column, the exact defect the width scaling exists to avoid.
/// Where the free part of the taskbar begins: after the task buttons, which Windows 11
/// centres, so the empty stretch is between them and the clock rather than the whole bar.
///
/// Without this the widget was anchored to the trailing end alone and drew straight over
/// the running applications' icons on a short taskbar — a portrait display's, measured at
/// 1080 pixels wide, where the centred buttons reach past the widget's leading edge.
#[cfg(windows)]
fn leading_edge(taskbar: HWND, rect: RECT) -> i32 {
    let mut right = rect.left;
    let _ = unsafe {
        EnumChildWindows(
            Some(taskbar),
            Some(task_buttons_right),
            LPARAM(std::ptr::from_mut(&mut right) as isize),
        )
    };
    right
}

#[cfg(windows)]
unsafe extern "system" fn task_buttons_right(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut class = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    if String::from_utf16_lossy(&class[..length]) == "MSTaskListWClass"
        && let Some(rect) = window_rect(hwnd)
    {
        let widest = lparam.0 as *mut i32;
        unsafe { *widest = (*widest).max(rect.right) };
    }
    TRUE
}

#[cfg(windows)]
fn taskbar_dpi(taskbar: &Taskbar) -> u32 {
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    match unsafe {
        GetDpiForMonitor(taskbar.monitor_handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y)
    } {
        Ok(()) if dpi_x > 0 => dpi_x,
        _ => 96,
    }
}

/// A layout width in CSS pixels, in the device pixels that display draws them at.
#[cfg(windows)]
fn scaled(logical: u32, dpi: u32) -> i32 {
    (f64::from(logical) * f64::from(dpi) / 96.0).round() as i32
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
    let widget = app.get_webview_window(&widget_label()).ok_or("taskbar widget window missing")?;
    let hwnd = widget.hwnd().map_err(|error| error.to_string())?;
    let rect = window_rect(hwnd).ok_or("unable to read taskbar widget bounds")?;
    Ok((
        PhysicalPosition::new(f64::from(rect.left), f64::from(rect.top)),
        PhysicalSize::new(f64::from(rect.right - rect.left), f64::from(rect.bottom - rect.top)),
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
        // A press elsewhere ends whatever pair was open. The release that should have
        // closed it can be lost outright — the system skips a hook that exceeds
        // `LowLevelHooksTimeout`, and the secure desktop and the lock screen both take the
        // button up with them — and a flag left standing would swallow the next release
        // anywhere on the machine, leaving that application in a drag it never started.
        WM_LBUTTONDOWN => (false, WidgetClickAction::Pass),
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

/// The clearance kept at the taskbar's leading edge, and half of it between the widget and
/// whatever ends the bar.
#[cfg(windows)]
const TASKBAR_MARGIN: u32 = 16;

/// What a secondary taskbar's clock is assumed to occupy, in CSS pixels, since it has no
/// window to measure.
#[cfg(windows)]
const SECONDARY_CLOCK_RESERVE: u32 = 160;

/// How many provider slots the renderer last asked for. Placement runs on a loop and has to
/// re-derive the width every tick — the display it docks to can change, and with it the
/// scaling the layout is drawn at — so the count outlives the call that reported it.
#[cfg(windows)]
static PROVIDER_SLOTS: AtomicU32 = AtomicU32::new(MIN_PROVIDER_SLOTS);

#[cfg(windows)]
fn provider_slots() -> u32 {
    PROVIDER_SLOTS.load(Ordering::Relaxed)
}

pub fn widget_width(provider_count: u32) -> u32 {
    let slots = provider_count.clamp(MIN_PROVIDER_SLOTS, MAX_PROVIDER_SLOTS);
    WIDGET_BASE_WIDTH.saturating_add(slots.saturating_mul(PROVIDER_SLOT_WIDTH))
}

fn docked_height(taskbar_height: i32) -> i32 {
    (taskbar_height - 6).max(20)
}

#[cfg(windows)]
pub fn set_widget_size(app: &tauri::AppHandle, provider_count: u32) -> Result<(), String> {
    PROVIDER_SLOTS
        .store(provider_count.clamp(MIN_PROVIDER_SLOTS, MAX_PROVIDER_SLOTS), Ordering::Relaxed);
    place_widget(app)
}

/// Parks the widget above the taskbar as an ordinary window. Every step is skipped when
/// it already holds, so the repositioning loop does not fight the window every tick.
#[cfg(windows)]
fn float_widget(app: &tauri::AppHandle, taskbar: Option<&Taskbar>) -> Result<(), String> {
    let widget = app.get_webview_window(&widget_label()).ok_or("taskbar widget window missing")?;
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

    // The fallback belongs on the display whose taskbar could not host it, not on whichever
    // one Windows calls primary: a status parked on the screen the user is not watching is
    // no better than one that vanished.
    let (origin, bounds, dpi) = match taskbar {
        Some(taskbar) => (
            PhysicalPosition::new(taskbar.monitor.left, taskbar.monitor.top),
            PhysicalSize::new(
                (taskbar.monitor.right - taskbar.monitor.left) as u32,
                (taskbar.monitor.bottom - taskbar.monitor.top) as u32,
            ),
            taskbar_dpi(taskbar),
        ),
        None => {
            let monitor = app
                .primary_monitor()
                .map_err(|error| error.to_string())?
                .ok_or("primary monitor missing")?;
            let scale = monitor.scale_factor();
            (*monitor.position(), *monitor.size(), (scale * 96.0).round() as u32)
        }
    };
    let size = PhysicalSize::new(scaled(widget_width(provider_slots()), dpi) as u32, WIDGET_HEIGHT);
    if widget.outer_size().map_err(|error| error.to_string())? != size {
        widget.set_size(size).map_err(|error| error.to_string())?;
    }
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
    use super::{WIDGET_HEIGHT, display_label, docked_height, widget_width};

    #[cfg(windows)]
    use super::scaled;

    #[cfg(windows)]
    use super::{WidgetClickAction, widget_click_transition};

    #[cfg(windows)]
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};

    #[test]
    fn the_widget_reserves_one_slot_for_every_reading_it_can_show() {
        // Two providers showing two windows each is the widest the interface goes, and the
        // taskbar clamps anything taller than this.
        assert!(widget_width(2) >= 2 * 200, "two provider columns must fit");
        const { assert!(WIDGET_HEIGHT >= 30, "the initial window must fit both rows before docking") };
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

    #[test]
    fn a_display_is_named_by_its_number_and_the_size_that_tells_two_screens_apart() {
        assert_eq!(
            display_label(r"\\.\DISPLAY2", true, 2752, 1152),
            "Display 2 (primary) — 2752 × 1152"
        );
        assert_eq!(display_label(r"\\.\DISPLAY1", false, 1080, 1920), "Display 1 — 1080 × 1920");
        assert_eq!(display_label("unnamed", false, 800, 600), "unnamed — 800 × 600");
    }

    #[cfg(windows)]
    #[test]
    fn a_layout_width_follows_the_scaling_of_the_display_it_docks_to() {
        assert_eq!(scaled(widget_width(2), 96), widget_width(2) as i32);
        assert_eq!(scaled(100, 120), 125, "a 125% taskbar draws 100 CSS pixels in 125 device ones");
        assert_eq!(scaled(100, 192), 200);
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
    fn a_press_elsewhere_releases_a_capture_whose_button_up_never_arrived() {
        let (captured, _) = widget_click_transition(false, WM_LBUTTONDOWN, true);
        assert!(captured, "the widget press is held");

        // The release is lost — a skipped hook, the secure desktop, the lock screen.
        let (captured, action) = widget_click_transition(captured, WM_LBUTTONDOWN, false);
        assert!(!captured, "a press elsewhere ends the abandoned pair");
        assert_eq!(action, WidgetClickAction::Pass);

        let (captured, action) = widget_click_transition(captured, WM_LBUTTONUP, false);
        assert!(!captured);
        assert_eq!(action, WidgetClickAction::Pass, "that click reaches its own window whole");
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
