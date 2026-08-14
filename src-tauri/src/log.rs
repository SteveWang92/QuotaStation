//! A small activity log on disk.
//!
//! Built executables are windowed applications with no console attached, and the status-line
//! bridge is a process that lives for a few milliseconds inside Claude Code, so neither can
//! report what it did anywhere a person could see. Both write here instead: one file in the
//! application data directory, appended to by whichever process is running.
//!
//! What goes in is the shape of what happened — which source answered, how many windows it
//! reported, why a read failed. Session content, prompts, credentials, and full provider
//! paths do not, exactly as with the diagnostics panel this complements.

use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};

const LOG_FILE: &str = "quotastation.log";
const PREVIOUS_LOG_FILE: &str = "quotastation.log.1";
/// Beyond this the current file is rolled over. Two files of this size are enough to hold
/// days of ordinary activity and never enough to matter on disk.
const MAX_BYTES: u64 = 512 * 1024;

pub fn log_path() -> Option<PathBuf> {
    crate::providers::claude::statusline::app_data_dir().map(|dir| dir.join(LOG_FILE))
}

/// Appends one line, with a timestamp and the process that wrote it. Failures are silent:
/// logging that cannot be written must never become a second fault to report.
pub fn write(message: impl AsRef<str>) {
    let safe_message = crate::sanitize::sanitize_error(message.as_ref(), "Activity unavailable");
    let _ = try_write(&safe_message);
}

fn try_write(message: &str) -> std::io::Result<()> {
    let Some(path) = log_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    if file.seek(SeekFrom::End(0))? > MAX_BYTES {
        drop(file);
        // The previous roll is replaced rather than kept, so the log stays bounded.
        let _ = std::fs::rename(&path, path.with_file_name(PREVIOUS_LOG_FILE));
        file = OpenOptions::new().create(true).append(true).open(&path)?;
    }
    writeln!(
        file,
        "{} [{}] {}",
        jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S"),
        std::process::id(),
        message
    )
}
