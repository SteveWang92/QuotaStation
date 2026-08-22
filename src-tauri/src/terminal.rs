//! Finding, and coming back to, the terminal a Claude Code turn was running in.
//!
//! A notification saying a turn has ended is only half an answer when six terminals are
//! open: the other half is getting back to the one that finished. Nothing in Claude Code
//! reports where it is being displayed, but the Stop hook is a child of it, so the window
//! can be found the way the process tree already records it — walk up from the hook through
//! `claude` and the shell until a process is reached that owns a visible top-level window.
//! That window is the terminal.
//!
//! It is the *window*, never the tab. Windows Terminal exposes no way to select one of its
//! tabs from outside the process: `wt focus-tab` addresses a window id and a tab index, and
//! nothing publishes which tab a given session is sitting in. Raising the right window and
//! leaving the last step to the user is the whole of what is possible, so it is all this
//! claims to do.

/// A window handle as it travels between the hook process and the application: a pointer,
/// carried as a number because it is written to a file in between.
pub type WindowHandle = isize;

use serde::{Deserialize, Serialize};

/// A recorded window and the process that owned it when the hook found it.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct TerminalTarget {
    pub window: WindowHandle,
    pub process_id: u32,
}

/// The terminal window the process that called this is running inside, if there is one.
///
/// Every failure along the way — no parent, no window, a console host with nothing visible —
/// costs the handle and nothing else. The notification is still raised; it simply has
/// nowhere to send a click.
#[cfg(windows)]
pub fn owning_window() -> Option<TerminalTarget> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, TRUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32, Process32First, Process32Next,
            TH32CS_SNAPPROCESS,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
        },
    };

    /// One pass over the process table, because walking the chain one lookup at a time
    /// would take a snapshot per generation.
    fn parents() -> Vec<(u32, u32)> {
        let mut pairs = Vec::new();
        let snapshot: HANDLE = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
            Ok(snapshot) => snapshot,
            Err(_) => return pairs,
        };
        let mut entry = PROCESSENTRY32 {
            dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
            ..Default::default()
        };
        if unsafe { Process32First(snapshot, &mut entry) }.is_ok() {
            loop {
                pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if unsafe { Process32Next(snapshot, &mut entry) }.is_err() {
                    break;
                }
            }
        }
        let _ = unsafe { CloseHandle(snapshot) };
        pairs
    }

    struct Search {
        process: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn visit(window: HWND, state: LPARAM) -> windows::core::BOOL {
        // SAFETY: the pointer is the one handed to EnumWindows on the line below, and
        // EnumWindows is synchronous, so it outlives every call.
        let search = unsafe { &mut *(state.0 as *mut Search) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
        // A titled, visible window: a console host keeps hidden ones, and a terminal that
        // has not drawn anything is not somewhere to send the user.
        if owner == search.process
            && unsafe { IsWindowVisible(window) }.as_bool()
            && unsafe { GetWindowTextLengthW(window) } > 0
        {
            search.found = Some(window);
            return windows::core::BOOL(0);
        }
        TRUE
    }

    fn window_of(process: u32) -> Option<HWND> {
        let mut search = Search { process, found: None };
        let _ =
            unsafe { EnumWindows(Some(visit), LPARAM(std::ptr::from_mut(&mut search) as isize)) };
        search.found
    }

    let pairs = parents();
    let mut current = std::process::id();
    // The chain from a Stop hook is short — hook, `claude`, the shell, the terminal — and
    // the bound is only there so a cycle in a corrupt process table cannot spin forever.
    for _ in 0..12 {
        if let Some(window) = window_of(current) {
            return Some(TerminalTarget { window: window.0 as WindowHandle, process_id: current });
        }
        let parent =
            pairs.iter().find(|(process, _)| *process == current).map(|(_, parent)| *parent);
        match parent {
            Some(parent) if parent != 0 && parent != current => current = parent,
            _ => return None,
        }
    }
    None
}

#[cfg(not(windows))]
pub fn owning_window() -> Option<TerminalTarget> {
    None
}

/// Brings a recorded window back to the front, and reports whether it is still there.
///
/// Windows refuses a foreground change from a process that is not already in the
/// foreground, and the toast being clicked belongs to the shell rather than to
/// QuotaStation. Attaching to the foreground thread's input queue for the length of the
/// call is the documented way through that, and it is bounded to exactly this one action.
#[cfg(windows)]
pub fn focus(target: TerminalTarget) -> bool {
    use windows::Win32::{
        Foundation::HWND,
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, SW_RESTORE,
            SetForegroundWindow, ShowWindow,
        },
    };

    let window = HWND(target.window as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(window)) }.as_bool() {
        return false;
    }
    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
    if owner != target.process_id {
        return false;
    }
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let this_thread = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != this_thread
            && AttachThreadInput(this_thread, foreground_thread, true).as_bool();
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let focused = SetForegroundWindow(window).as_bool();
        if attached {
            let _ = AttachThreadInput(this_thread, foreground_thread, false);
        }
        focused
    }
}

#[cfg(not(windows))]
pub fn focus(_target: TerminalTarget) -> bool {
    false
}
