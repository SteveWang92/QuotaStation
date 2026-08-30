//! How QuotaStation is started, and whether that start opens a window.
//!
//! A start Windows performs at logon should leave the machine as the user left it: the tray
//! icon and the taskbar status appear, and no dashboard takes the foreground in the middle
//! of everything else coming up. A start the user performed — from the Start menu, a desktop
//! shortcut, or the executable itself — is a request to look at the dashboard, so it opens
//! one.
//!
//! The two are told apart by an argument, because nothing else distinguishes them: the logon
//! entry is registered with [`BACKGROUND_ARG`] and a manual launch carries no arguments at
//! all.

/// Starts QuotaStation to the tray with no dashboard window.
pub const BACKGROUND_ARG: &str = "--background";

/// The name Windows keys the logon entry on.
///
/// The autostart plugin writes the entry under Tauri's package name and the NSIS
/// uninstaller deletes `${PRODUCTNAME}`; both are `productName` in `tauri.conf.json`. The
/// uninstall path runs before Tauri does and cannot ask it, so the name is repeated here.
#[cfg(windows)]
pub const LOGON_ENTRY_NAME: &str = "QuotaStation";

/// Whether the logon entry exists and starts *this* executable.
///
/// Two copies of QuotaStation can exist on one machine and they share the single entry
/// name, so an entry naming the other copy is not this one's to record or restore.
#[cfg(windows)]
pub fn logon_entry_is_ours() -> bool {
    let Some(command) = registered_command(LOGON_ENTRY_NAME) else { return false };
    let Ok(executable) = std::env::current_exe() else { return false };
    command.contains(&executable.display().to_string())
}

/// Whether this process was asked to start without a window.
pub fn requested() -> bool {
    std::env::args_os().any(|argument| argument == BACKGROUND_ARG)
}

/// Brings an already-registered logon entry up to date with [`BACKGROUND_ARG`].
///
/// The autostart plugin registers the argument with every new entry, but it reports only
/// whether an entry exists, so one written by an earlier version would go on opening a
/// window at every logon until the setting was toggled off and on again. Rewriting it is
/// safe only where the entry already names this executable: two copies of QuotaStation can
/// exist on one machine, and the running one must not quietly claim the other's logon slot.
#[cfg(windows)]
pub fn refresh_logon_entry<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    if !manager.is_enabled().unwrap_or(false) {
        return;
    }
    let Some(command) = registered_command(&app.package_info().name) else { return };
    if command.contains(BACKGROUND_ARG) {
        return;
    }
    let Ok(executable) = std::env::current_exe() else { return };
    if !command.contains(&executable.display().to_string()) {
        return;
    }
    match manager.enable() {
        Ok(()) => crate::log::write("start with Windows now starts without a window"),
        Err(error) => crate::log::write(format!("could not update the logon entry: {error}")),
    }
}

#[cfg(not(windows))]
pub fn refresh_logon_entry<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {}

/// The command Windows runs at logon for `name`, or `None` when there is no entry.
#[cfg(windows)]
fn registered_command(name: &str) -> Option<String> {
    use windows::{
        Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW},
        core::PCWSTR,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let key = wide(RUN_KEY);
    let value = wide(name);
    // The value is read into a buffer sized for any plausible command line; a longer one
    // simply means the entry is not one QuotaStation wrote.
    let mut buffer = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let characters = (size as usize / 2).saturating_sub(1).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..characters]))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value).encode_wide().chain(std::iter::once(0)).collect()
}
