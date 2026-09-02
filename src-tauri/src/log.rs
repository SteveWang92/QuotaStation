//! A small activity log on disk.
//!
//! Built executables are windowed applications with no console attached, and the status-line
//! bridge is a process that lives for a few milliseconds inside Claude Code, so neither can
//! report what it did anywhere a person could see. Both write here instead: one file in the
//! application data directory, appended to by whichever process is running.
//!
//! What goes in is the shape of what happened — which source answered, how many windows it
//! reported, why a read failed, which window the user opened, which setting they changed.
//! Session content, prompts, credentials, and full provider paths do not, exactly as with
//! the diagnostics panel this complements: every line passes through [`crate::sanitize`],
//! which redacts anything path-shaped on its way in.
//!
//! It is written to be read after the fact by someone who was not watching, so it records
//! what worked as well as what did not. A read that answered normally is what makes the one
//! that did not stand out.

use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};

const LOG_FILE: &str = "quotastation.log";
const PREVIOUS_LOG_FILE: &str = "quotastation.log.1";
/// Beyond this the current file is rolled over and the roll before it is dropped. This pair
/// is the whole retention policy: nothing is dated, nothing is swept on a timer, and the two
/// files together cannot outgrow it. The size is measured rather than guessed: an active
/// coding session costs around 200 KB an hour once every source above is recorded, so this
/// holds roughly a fortnight of ordinary use and the roll behind it holds the fortnight
/// before. It is still nothing beside the usage database in the same folder.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

pub fn log_path() -> Option<PathBuf> {
    crate::providers::claude::statusline::app_data_dir().map(|dir| dir.join(LOG_FILE))
}

/// Appends one line, with a timestamp and the process that wrote it. Failures are silent:
/// logging that cannot be written must never become a second fault to report.
pub fn write(message: impl AsRef<str>) {
    let safe_message = crate::sanitize::sanitize_log(message.as_ref());
    if safe_message.is_empty() {
        return;
    }
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
    // The application and the status-line bridge append to this file at the same time, so
    // the line is assembled first and handed over in one write: formatting straight into the
    // file writes it in pieces, and the two processes interleave halfway through a line.
    let line = format!(
        "{} [{}] {}\n",
        jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S"),
        std::process::id(),
        message
    );
    file.write_all(line.as_bytes())
}
