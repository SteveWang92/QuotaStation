//! What has to be handed to Windows rather than done here: opening a folder or a page,
//! selecting a file in Explorer, the desktop shortcut, and the logon entry.

use std::path::{Path, PathBuf};

use tauri::Manager;

#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

use crate::{log, sanitize};

/// Whether the activity log can be revealed without exposing its path to the renderer.
///
/// A built executable has no console: neither the application nor the status-line bridge
/// can report what it did anywhere a person could see, so both write to this file and the
/// diagnostics panel points at it.
#[tauri::command]
pub(crate) fn get_log_available() -> bool {
    log::log_path().is_some()
}

#[tauri::command]
pub(crate) fn reveal_log_file() -> Result<(), String> {
    log::write("activity log revealed in Explorer");
    let path = log::log_path().ok_or_else(|| "No application data directory.".to_string())?;
    // Selecting the file rather than opening it: the log is read with whatever the user
    // prefers, and a missing file still lands them in the right folder.
    select_in_explorer(&path).map_err(|error| error.to_string())?;
    Ok(())
}

/// Opens QuotaStation's data directory without returning its machine-specific path to the
/// renderer. The directory already exists by the time Settings can be opened.
#[tauri::command]
pub(crate) fn open_data_folder(app: tauri::AppHandle) -> Result<(), String> {
    log::write("data folder opened in Explorer");
    let path = app.path().app_data_dir().map_err(|error| error.to_string())?;
    open_in_explorer(&path).map_err(|error| error.to_string())
}

/// Opens the public release page in the default browser. The URL is fixed in the core so
/// the renderer cannot turn this narrow action into an arbitrary shell launch.
#[tauri::command]
pub(crate) fn open_latest_release() -> Result<(), String> {
    log::write("release page opened in the browser");
    open_with_explorer("https://github.com/SteveWang92/QuotaStation/releases/latest")
        .map_err(|error| error.to_string())
}

/// Shows a file selected in Explorer.
///
/// Explorer parses its own raw command line and ignores a `/select,` token that starts
/// with a quote. Rust quotes a whole argument that contains a space, which a user profile
/// name or a typed filename easily does, so the path is quoted inside the argument here
/// instead.
fn select_in_explorer(path: &Path) -> std::io::Result<()> {
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(format!("/select,\"{}\"", path.display()));
    }
    #[cfg(not(windows))]
    command.arg(format!("/select,{}", path.display()));
    command.spawn()?;
    Ok(())
}

fn open_in_explorer(path: &Path) -> std::io::Result<()> {
    open_with_explorer(path.as_os_str())
}

fn open_with_explorer(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<()> {
    std::process::Command::new("explorer.exe").arg(target).spawn()?;
    Ok(())
}

/// Reveals the export the user just created without opening its contents.
#[tauri::command]
pub(crate) fn reveal_export_file(path: String) -> Result<(), String> {
    select_in_explorer(Path::new(&path))
        .map_err(|_| "The exported file could not be shown in Explorer.".to_string())?;
    Ok(())
}

#[cfg(windows)]
fn write_desktop_shortcut(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let shortcut_path =
        app.path().desktop_dir().map_err(|error| error.to_string())?.join("QuotaStation.lnk");
    let mut shortcut = mslnk::ShellLink::new(&executable).map_err(|error| error.to_string())?;
    if let Some(working_directory) = executable.parent() {
        shortcut.set_working_dir(Some(working_directory.to_string_lossy().into_owned()));
    }
    shortcut.set_icon_location(Some(executable.to_string_lossy().into_owned()));
    shortcut.set_name(Some("QuotaStation".to_string()));
    shortcut.create_lnk(&shortcut_path).map_err(|error| error.to_string())?;
    Ok(shortcut_path)
}

#[cfg(not(windows))]
fn write_desktop_shortcut(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Err("Desktop shortcuts are currently supported on Windows only.".to_string())
}

/// Whether Windows starts QuotaStation on sign-in. The plugin owns the registration, so
/// this reports what it holds rather than a copy kept in the settings file.
#[tauri::command]
pub(crate) fn get_autostart(app: tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub(crate) fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    log::write(format!(
        "start with Windows switched {}{}",
        if enabled { "on" } else { "off" },
        match &result {
            Ok(()) => String::new(),
            Err(error) => format!(", which failed: {error}"),
        }
    ));
    result.map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "Start-with-Windows update failed")
    })?;
    Ok(manager.is_enabled().unwrap_or(enabled))
}

/// Whether a typed shared-folder path already names a folder.
///
/// The path is hand-entered, so it is a trust boundary: it may name a file, a folder that
/// does not exist yet, or nothing reachable at all. The settings page asks before it saves
/// so it can offer to create a missing folder rather than storing a path that will fail
/// quietly on every export afterwards.
#[tauri::command]
pub(crate) fn shared_folder_exists(path: String) -> Result<bool, String> {
    let path = std::path::Path::new(path.trim());
    if path.as_os_str().is_empty() {
        return Err("Enter a folder path.".to_string());
    }
    if path.is_file() {
        return Err("That path is a file, not a folder.".to_string());
    }
    Ok(path.is_dir())
}

/// Creates the folder the user confirmed, parents included.
#[tauri::command]
pub(crate) fn create_shared_folder(path: String) -> Result<(), String> {
    log::write("shared usage folder created");
    std::fs::create_dir_all(path.trim()).map_err(|error| {
        sanitize::sanitize_error(&error.to_string(), "The folder could not be created")
    })
}

/// Puts a shortcut on the desktop. The location is the user's own desktop, so the path is
/// neither reported back nor worth reporting.
#[tauri::command]
pub(crate) fn create_desktop_shortcut(app: tauri::AppHandle) -> Result<(), String> {
    log::write("desktop shortcut requested");
    write_desktop_shortcut(&app)
        .map(|_| ())
        .map_err(|error| sanitize::sanitize_error(&error, "Desktop shortcut creation failed"))
}
