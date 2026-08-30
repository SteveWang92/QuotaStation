//! Putting back what the uninstaller took away, and only that.
//!
//! Uninstalling QuotaStation removes three things that live outside its own data: Claude
//! Code's status line and `Stop` hook, which [`crate::run_uninstall_cleanup`] deletes, and
//! the Windows logon entry, which the NSIS uninstaller deletes itself. The application data
//! beside them survives unless the user asks for it to go, so a reinstall already comes back
//! to its old history, theme, widget and notification settings — and then contradicts itself
//! by showing those three integrations switched off.
//!
//! They are not restored from a copy of the settings, because a copy cannot tell "the user
//! turned this off" from "the uninstaller took this away", and a launch that re-registers a
//! command the user removed by hand is worse than one that forgets. Instead the uninstall
//! writes down what was actually on at that moment, and the next start consumes that note
//! once. No other start touches another program's settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::providers::claude::{claude_home, notifications, statusline};

const FILE: &str = "restore-integrations.json";

/// What was switched on when QuotaStation was uninstalled.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Removed {
    claude_status_line: bool,
    claude_notifications: bool,
    start_with_windows: bool,
}

impl Removed {
    fn any(&self) -> bool {
        self.claude_status_line || self.claude_notifications || self.start_with_windows
    }
}

fn note_path() -> Option<PathBuf> {
    statusline::app_data_dir().map(|dir| dir.join(FILE))
}

/// Writes down the integrations an uninstall is about to remove.
///
/// Called before anything is removed, so it records what was true rather than what is
/// wanted. Selecting **Delete app data** deletes this note along with everything else,
/// which is the right answer: an uninstall that keeps nothing has nothing to restore.
pub fn record_uninstall() {
    let removed = Removed {
        claude_status_line: statusline::installed(),
        claude_notifications: notifications::installed(),
        start_with_windows: logon_entry_is_ours(),
    };
    if !removed.any() {
        return;
    }
    let Some(path) = note_path() else { return };
    match serde_json::to_vec_pretty(&removed).map_err(std::io::Error::other) {
        Ok(content) => match std::fs::write(&path, content) {
            Ok(()) => crate::log::write("uninstall recorded the integrations to restore"),
            Err(error) => {
                crate::log::write(format!("uninstall could not record the integrations: {error}"));
            }
        },
        Err(error) => {
            crate::log::write(format!("uninstall could not record the integrations: {error}"));
        }
    }
}

/// Re-registers whatever the last uninstall removed, once.
///
/// The note is deleted whether or not every item could be restored. A status line that
/// something else has claimed in the meantime is the user's to sort out, and retrying it at
/// every start would be the silent meddling this whole path avoids.
pub fn restore_after_reinstall<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let Some(path) = note_path() else { return };
    let Ok(content) = std::fs::read(&path) else { return };
    let removed: Removed = match serde_json::from_slice(&content) {
        Ok(removed) => removed,
        Err(error) => {
            crate::log::write(format!("the recorded integrations could not be read: {error}"));
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);

    // Claude Code may have been uninstalled too. Installing into a settings file in a
    // directory that no longer exists would leave a stray configuration for a program that
    // is not there.
    let claude_present = claude_home().is_some_and(|home| home.is_dir());
    if removed.claude_status_line && claude_present {
        report("Claude Code status line", statusline::install());
    }
    if removed.claude_notifications && claude_present {
        report("Claude Code notification hook", notifications::install());
    }
    if removed.start_with_windows {
        use tauri_plugin_autostart::ManagerExt;
        report("start with Windows", app.autolaunch().enable().map_err(anyhow::Error::from));
    }
}

fn report(name: &str, result: anyhow::Result<()>) {
    match result {
        Ok(()) => crate::log::write(format!("the {name} was restored after reinstalling")),
        Err(error) => {
            crate::log::write(format!("the {name} could not be restored: {error:#}"));
        }
    }
}

#[cfg(windows)]
fn logon_entry_is_ours() -> bool {
    crate::autostart::logon_entry_is_ours()
}

#[cfg(not(windows))]
fn logon_entry_is_ours() -> bool {
    false
}
