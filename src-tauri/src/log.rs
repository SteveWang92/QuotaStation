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
//! that did not stand out. What it does not record is that same normal read a second time:
//! a line whose content has not changed is counted rather than repeated, which is the
//! difference between a session's worth of identical reads and one line saying it read the
//! same thing four hundred times.

use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const LOG_FILE: &str = "quotastation.log";
const PREVIOUS_LOG_FILE: &str = "quotastation.log.1";
/// Beyond this the current file is rolled over and the roll before it is dropped. This pair
/// is the whole retention policy: nothing is dated, nothing is swept on a timer, and the two
/// files together cannot outgrow it. The size is measured rather than guessed: an active
/// coding session costs around 90 KB an hour once repeated lines are collapsed, so this holds
/// several weeks of ordinary use and the roll behind it holds the weeks before. It is still
/// nothing beside the usage database in the same folder.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// How long an unchanged line stays suppressed before it is written again anyway. Without a
/// ceiling a quiet application reads exactly like a stopped one, and saying which is the
/// log's job. Measured against a real log, this ceiling costs under three per cent of what
/// the collapsing saves.
const REPEAT_CEILING: Duration = Duration::from_secs(5 * 60);
/// A bound on the line shapes tracked at once, cleared wholesale when it is reached. Most
/// shapes are fixed statements this code can write, but two sources add one shape at a time —
/// an error text that varies, and the date range a query names — so a process that runs for
/// weeks needs a bound. This one is far above what a real log reaches in a day.
const MAX_TRACKED_SHAPES: usize = 1024;

/// Per shape: the content last written, when it was written, and how many identical lines
/// have been skipped since.
type Repeats = HashMap<String, (String, Instant, u32)>;

fn repeats() -> &'static Mutex<Repeats> {
    static REPEATS: OnceLock<Mutex<Repeats>> = OnceLock::new();
    REPEATS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn log_path() -> Option<PathBuf> {
    crate::providers::claude::statusline::app_data_dir().map(|dir| dir.join(LOG_FILE))
}

/// Appends one line, with a timestamp and the process that wrote it, unless it says exactly
/// what the last line of its kind already said. Failures are silent: logging that cannot be
/// written must never become a second fault to report.
pub fn write(message: impl AsRef<str>) {
    let safe_message = crate::sanitize::sanitize_log(message.as_ref());
    if safe_message.is_empty() {
        return;
    }
    // The lock is held across the append so the file's order is the order the decisions were
    // made in. The writes are a few a second and the append is small.
    let mut state = repeats().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(line) = decide(&mut state, &safe_message, Instant::now()) {
        let _ = try_write(&line);
    }
}

/// Whether this line is written, and what it says when it is. A line repeating its own last
/// content is skipped and counted; the next one that does not repeat carries the count, so
/// the skipped lines stay visible as a number rather than disappearing.
fn decide(state: &mut Repeats, message: &str, now: Instant) -> Option<String> {
    let shape = shape_of(message);
    let content = content_of(message);
    let Some((last_content, written_at, skipped)) = state.get_mut(&shape) else {
        // Every distinct shape is one entry, so the map only grows without bound through the
        // error texts among them. Dropping the lot is enough: the worst it costs is one
        // repeated line for each shape still in use.
        if state.len() >= MAX_TRACKED_SHAPES {
            state.clear();
        }
        state.insert(shape, (content, now, 0));
        return Some(message.to_string());
    };
    if *last_content == content && now.duration_since(*written_at) < REPEAT_CEILING {
        *skipped += 1;
        return None;
    }
    let line = match *skipped {
        0 => message.to_string(),
        count => format!("{message} [+{count} unchanged]"),
    };
    *last_content = content;
    *written_at = now;
    *skipped = 0;
    Some(line)
}

/// The shape of a line: what it was about, with what it found masked out. Two lines of the
/// same shape are the same recurring statement, and the first `": "` is where one turns into
/// the other — everything before it says what was asked and stays literal, everything after
/// says what came back and is masked. That is what keeps the two windows read one after the
/// other, and the two date ranges the dashboard queries in turn, apart: paired as one shape
/// they would report a different answer every time and never settle.
///
/// Two lines that differ only in a word rather than a number are two shapes, so a state that
/// flaps between them is written once each and then counted. The ceiling is what keeps it
/// visible: both come back every five minutes, each carrying how often it recurred.
fn shape_of(message: &str) -> String {
    let content = content_of(message);
    let Some(reported) = content.find(": ") else { return content };
    let (asked, found) = content.split_at(reported + 2);
    format!("{asked}{}", mask_numbers(found, |_| true))
}

/// The same line with only its durations masked. How long a read took is not a change worth
/// writing the line again for; everything else it reports is.
fn content_of(message: &str) -> String {
    mask_numbers(message, |after| after.starts_with("ms"))
}

fn mask_numbers(message: &str, masked: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find(|character: char| character.is_ascii_digit()) {
        out.push_str(&rest[..start]);
        let digits = &rest[start..];
        let end =
            digits.find(|character: char| !character.is_ascii_digit()).unwrap_or(digits.len());
        let (run, after) = digits.split_at(end);
        out.push_str(if masked(after) { "#" } else { run });
        rest = after;
    }
    out.push_str(rest);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_repeating_its_own_content_is_counted_rather_than_written() {
        let mut state = Repeats::new();
        let now = Instant::now();
        let one_day = "query daily usage for all providers: 1 day(s), 1 model(s)";
        let two_days = "query daily usage for all providers: 2 day(s), 1 model(s)";

        assert_eq!(decide(&mut state, one_day, now), Some(one_day.to_string()));
        assert_eq!(decide(&mut state, one_day, now), None);
        assert_eq!(decide(&mut state, one_day, now), None);
        assert_eq!(decide(&mut state, two_days, now), Some(format!("{two_days} [+2 unchanged]")));
    }

    #[test]
    fn only_the_duration_changing_is_not_a_change() {
        let mut state = Repeats::new();
        let now = Instant::now();

        assert!(decide(&mut state, "claude history parsed in 136ms: 31 day(s)", now).is_some());
        assert_eq!(decide(&mut state, "claude history parsed in 55ms: 31 day(s)", now), None);
        assert_eq!(
            decide(&mut state, "claude history parsed in 55ms: 32 day(s)", now),
            Some("claude history parsed in 55ms: 32 day(s) [+1 unchanged]".to_string())
        );
    }

    #[test]
    fn two_lines_alternating_do_not_suppress_each_other() {
        let mut state = Repeats::new();
        let now = Instant::now();
        let five_hour = "claude window 5-hour window: used Some(23.0)%";
        let weekly = "claude window Weekly window: used Some(19.0)%";

        assert!(decide(&mut state, five_hour, now).is_some());
        assert!(decide(&mut state, weekly, now).is_some());
        assert_eq!(decide(&mut state, five_hour, now), None);
        assert_eq!(decide(&mut state, weekly, now), None);
    }

    #[test]
    fn two_queries_asking_different_things_do_not_suppress_each_other() {
        let mut state = Repeats::new();
        let now = Instant::now();
        let today = "query daily usage 2026-09-03..2026-09-03: 1 day(s), 1 model(s)";
        let yesterday = "query daily usage 2026-09-02..2026-09-02: 1 day(s), 1 model(s)";

        assert!(decide(&mut state, today, now).is_some());
        assert!(decide(&mut state, yesterday, now).is_some());
        assert_eq!(decide(&mut state, today, now), None);
        assert_eq!(decide(&mut state, yesterday, now), None);
    }

    #[test]
    fn an_unchanged_line_is_written_again_once_the_ceiling_passes() {
        let mut state = Repeats::new();
        let now = Instant::now();
        let published = "snapshot published, Quota healthy: claude 2 window(s)";

        assert!(decide(&mut state, published, now).is_some());
        assert_eq!(decide(&mut state, published, now), None);
        assert_eq!(
            decide(&mut state, published, now + REPEAT_CEILING),
            Some(format!("{published} [+1 unchanged]"))
        );
    }
}
