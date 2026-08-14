//! Claude quota read from Claude Code's own status line.
//!
//! Since Claude Code 2.1.80 the JSON it hands a status-line command carries
//! `rate_limits.five_hour` and `rate_limits.seven_day`, each with the percentage consumed
//! and the epoch second the window resets. That is the same pair of windows Anthropic's
//! OAuth usage endpoint reports, delivered locally: no credential is presented, nothing
//! leaves the machine, and no rate limit is shared with Claude Code's own usage display.
//!
//! Claude Code only hands that JSON to the command configured as its status line, so
//! QuotaStation registers itself as that command. The registered process does two things
//! and exits: it writes the two windows to the application data directory, and it prints a
//! status line. It never starts the interface, and a malformed or unexpected payload only
//! costs the reading — the status line still prints, because a monitor must not be able to
//! break the client it monitors.
//!
//! The windows are only as recent as the last Claude Code turn that rendered a status
//! line, which is why the session-log window stays in place as the fallback: it is derived
//! from files that are written whether or not this bridge is installed.

use std::{
    env,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::domain::{Freshness, LimitKind, LimitWindow, WindowSource};

use super::{FIVE_HOUR_WINDOW_MINS, SEVEN_DAY_WINDOW_MINS, claude_home};

/// The argument that turns this executable into the status-line command. It is deliberately
/// specific: a stray argument must never be mistaken for a request to write to stdout
/// instead of opening the interface.
pub const BRIDGE_ARG: &str = "--claude-statusline";

/// Where the bridge leaves what it read, inside QuotaStation's own application data
/// directory. Several Claude Code sessions can render a status line at once, so the file is
/// replaced atomically rather than appended to.
const CACHE_FILE: &str = "claude-status-line.json";

/// A reading older than this describes windows that have long since restarted, so it is
/// treated as absent rather than as very stale data.
const MAX_READING_AGE_SECS: i64 = 14 * 24 * 60 * 60;

/// The subset of Claude Code's status-line payload QuotaStation reads. Every field is
/// optional: the payload grows between releases, and a field that is missing means the
/// window is unknown, never that the reading failed.
#[derive(Deserialize)]
struct StatusLineInput {
    rate_limits: Option<RateLimits>,
    model: Option<Model>,
}

#[derive(Deserialize)]
struct Model {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct RateLimits {
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
struct Bucket {
    /// Percentage of the window consumed, from 0 to 100.
    used_percentage: Option<f64>,
    /// Epoch seconds at which the window restarts.
    resets_at: Option<i64>,
}

/// What the bridge stored, and when. The observation time is kept so a reading left behind
/// by a Claude Code session that ended days ago can be recognised as such.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
struct Reading {
    observed_at: i64,
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
}

/// The quota windows Claude Code last reported to the status line, newest reading first.
///
/// A window whose restart has already passed is dropped rather than shown as expired: the
/// session-log window covers the five-hour case, and a seven-day window that has restarted
/// says nothing about the one now running.
pub fn read_windows() -> Result<Vec<LimitWindow>> {
    let Some(reading) = load_reading()? else { return Ok(Vec::new()) };
    windows_from(&reading, jiff::Timestamp::now().as_second())
}

/// When the bridge last recorded a reading, for the interface to explain how current the
/// windows are.
pub fn last_reading_at() -> Option<i64> {
    load_reading().ok().flatten().map(|reading| reading.observed_at)
}

fn windows_from(reading: &Reading, now: i64) -> Result<Vec<LimitWindow>> {
    if now - reading.observed_at > MAX_READING_AGE_SECS {
        return Ok(Vec::new());
    }
    if reading.observed_at <= 0 || reading.observed_at > now + 300 {
        bail!("schema_incompatible: invalid status-line observation time");
    }
    let mut windows = Vec::new();
    for (bucket, kind, minutes) in [
        (reading.five_hour, LimitKind::Primary, FIVE_HOUR_WINDOW_MINS),
        (reading.seven_day, LimitKind::Secondary, SEVEN_DAY_WINDOW_MINS),
    ] {
        let Some(bucket) = bucket else { continue };
        let Some(used_percent) = bucket.used_percentage else {
            bail!("schema_incompatible: status-line bucket omitted used percentage");
        };
        let Some(resets_at) = bucket.resets_at else {
            bail!("schema_incompatible: status-line bucket omitted reset time");
        };
        if !used_percent.is_finite() || !(0.0..=100.0).contains(&used_percent) {
            bail!("schema_incompatible: invalid status-line percentage");
        }
        if resets_at <= now {
            continue;
        }
        if resets_at > now + minutes * 60 * 2 {
            bail!("schema_incompatible: invalid status-line reset time");
        }
        windows.push(LimitWindow {
            kind,
            label: kind.window_label(Some(minutes)),
            used_percent: Some(used_percent),
            remaining_percent: Some((100.0 - used_percent).clamp(0.0, 100.0)),
            window_duration_mins: Some(minutes),
            resets_at: Some(resets_at),
            source: WindowSource::StatusLine,
            observed_at: reading.observed_at,
            freshness: Freshness::Fresh,
        });
    }
    if reading.five_hour.is_some() || reading.seven_day.is_some() {
        return Ok(windows);
    }
    bail!("schema_incompatible: status-line rate limits contained no known buckets")
}

fn load_reading() -> Result<Option<Reading>> {
    let Some(path) = cache_path() else { return Ok(None) };
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read status-line cache"),
    };
    serde_json::from_str(&content)
        .map(Some)
        .context("schema_incompatible: decode status-line cache")
}

fn cache_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(CACHE_FILE))
}

/// QuotaStation's application data directory, resolved the same way Tauri resolves it, so
/// the short-lived bridge process and the running application agree on one file without the
/// bridge having to start Tauri to ask.
pub fn app_data_dir() -> Option<PathBuf> {
    const IDENTIFIER: &str = "me.stevewang.quotastation";
    #[cfg(windows)]
    {
        env::var_os("APPDATA").map(|roaming| PathBuf::from(roaming).join(IDENTIFIER))
    }
    #[cfg(not(windows))]
    {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .map(|data| data.join(IDENTIFIER))
    }
}

/// Runs as Claude Code's status-line command when this executable was started with
/// [`BRIDGE_ARG`], and reports whether it did.
///
/// Everything here is best effort. Claude Code renders whatever this prints, so a failure
/// to store the reading must still leave a status line behind.
pub fn run_bridge_if_requested() -> bool {
    if !env::args_os().any(|argument| argument == BRIDGE_ARG) {
        return false;
    }
    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);
    let input = serde_json::from_str::<StatusLineInput>(&payload).ok();
    let limits = input.as_ref().and_then(|input| input.rate_limits.as_ref());
    // This process has no console and no window, so the log is the only place it can say
    // whether Claude Code ran it and what the payload contained. Sizes and field presence
    // only: the payload also carries the session's own working context.
    crate::log::write(format!(
        "status line bridge ran: {} bytes of input, parsed {}, rate_limits {}",
        payload.len(),
        if input.is_some() { "yes" } else { "no" },
        match limits {
            None => "absent".to_string(),
            Some(limits) => format!(
                "five_hour {} seven_day {}",
                if limits.five_hour.is_some() { "present" } else { "absent" },
                if limits.seven_day.is_some() { "present" } else { "absent" },
            ),
        }
    ));
    if let Some(limits) = limits {
        let now = jiff::Timestamp::now().as_second();
        let reading = Reading {
            observed_at: now,
            five_hour: limits.five_hour,
            seven_day: limits.seven_day,
        };
        match windows_from(&reading, now) {
            Ok(_) if store_reading(&reading).is_ok() => {
                crate::log::write("status line reading stored");
            }
            Ok(_) => crate::log::write("status line reading not stored"),
            Err(_) => {
                // Persist only the normalized quota subset, not the source payload. The
                // running app can then report schema_incompatible and preserve its LKG.
                let _ = store_reading(&reading);
                crate::log::write("status line schema incompatible");
            }
        }
    } else if input.is_some()
        && let Some(cache) = cache_path()
    {
        let _ = std::fs::remove_file(cache);
    }
    let model = input
        .as_ref()
        .and_then(|input| input.model.as_ref())
        .and_then(|model| model.display_name.clone());
    println!(
        "{}",
        status_line(
            model.as_deref(),
            limits.and_then(|limits| limits.five_hour),
            limits.and_then(|limits| limits.seven_day),
        )
    );
    true
}

/// What Claude Code shows while the bridge is installed. It reports the same two windows
/// QuotaStation now has, so installing the bridge adds a status line rather than taking the
/// row for nothing.
fn status_line(model: Option<&str>, five_hour: Option<Bucket>, seven_day: Option<Bucket>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        parts.push(model.to_string());
    }
    for (bucket, label) in [(five_hour, "5h"), (seven_day, "7d")] {
        if let Some(used) = bucket.and_then(|bucket| bucket.used_percentage) {
            parts.push(format!("{label} {:.0}% used", used.clamp(0.0, 100.0)));
        }
    }
    if parts.is_empty() {
        // Claude Code hands out no rate limits on API-key and enterprise sign-ins, and an
        // empty line would read as a broken status line rather than an absent quota.
        return "QuotaStation".to_string();
    }
    parts.join(" · ")
}

fn store_reading(reading: &Reading) -> Result<()> {
    let path = cache_path().context("resolve the application data directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create the application data directory")?;
    }
    // Concurrent Claude Code sessions render status lines independently, so the file is
    // published by rename: a reader never sees a half-written reading.
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staging, serde_json::to_string(reading)?)?;
    std::fs::rename(&staging, &path).map_err(|error| {
        let _ = std::fs::remove_file(&staging);
        error
    })?;
    Ok(())
}

/// Whether Claude Code is configured to hand its quota to QuotaStation, and what stands in
/// the way when it is not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub installed: bool,
    /// A status line belonging to something else. Replacing it is the user's call, so it is
    /// reported rather than overwritten.
    pub has_foreign_command: bool,
    pub last_reading_at: Option<i64>,
    /// Claude Code is running, but only in hosts that render their own interface instead of
    /// a status line. Nothing the bridge does can produce a reading from those sessions, so
    /// an installation that looks inert has to be able to say that is why.
    pub desktop_only_sessions: bool,
}

pub fn bridge_status() -> BridgeStatus {
    let configured = configured_command();
    let ours = configured.as_deref().is_some_and(is_bridge_command);
    BridgeStatus {
        installed: ours,
        has_foreign_command: configured.is_some() && !ours,
        last_reading_at: last_reading_at(),
        desktop_only_sessions: super::sessions::live_sessions().desktop_only(),
    }
}

fn settings_path() -> Result<PathBuf> {
    Ok(claude_home()
        .context("locate the Claude Code configuration directory")?
        .join("settings.json"))
}

fn load_settings(path: &Path) -> Result<serde_json::Value> {
    Ok(load_settings_with_source(path)?.0)
}

fn load_settings_with_source(path: &Path) -> Result<(serde_json::Value, Option<Vec<u8>>)> {
    let source = match std::fs::read(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((serde_json::Value::Object(serde_json::Map::new()), None));
        }
        Err(error) => return Err(error).context("read the Claude Code settings"),
    };
    let content = std::str::from_utf8(&source).context("decode the Claude Code settings")?;
    if content.trim().is_empty() {
        return Ok((serde_json::Value::Object(serde_json::Map::new()), Some(source)));
    }
    let settings = serde_json::from_str(content).context("parse the Claude Code settings")?;
    Ok((settings, Some(source)))
}

fn configured_command() -> Option<String> {
    command_in(&load_settings(&settings_path().ok()?).ok()?)
}

fn command_in(settings: &serde_json::Value) -> Option<String> {
    settings.get("statusLine")?.get("command")?.as_str().map(str::to_string)
}

fn is_bridge_command(command: &str) -> bool {
    command.contains(BRIDGE_ARG)
}

/// The command Claude Code has to run. The executable is quoted because it lives under a
/// path with spaces on an ordinary Windows installation.
pub fn bridge_command() -> Result<String> {
    let executable = env::current_exe().context("locate the QuotaStation executable")?;
    Ok(format!("\"{}\" {BRIDGE_ARG}", executable.display()))
}

/// Registers QuotaStation as Claude Code's status-line command, leaving every other setting
/// as it was. A status line belonging to something else is never replaced.
pub fn install() -> Result<()> {
    install_into(&settings_path()?, &bridge_command()?)
}

/// Removes the status line again, and only ever the one QuotaStation installed.
pub fn remove() -> Result<()> {
    remove_from(&settings_path()?)?;
    if let Some(cache) = cache_path() {
        let _ = std::fs::remove_file(cache);
    }
    Ok(())
}

fn install_into(path: &Path, command: &str) -> Result<()> {
    let (mut settings, source) = load_settings_with_source(path)?;
    if let Some(existing) = command_in(&settings)
        && !is_bridge_command(&existing)
    {
        bail!(
            "Claude Code already runs its own status line. Remove that status line first, \
             then install this one."
        );
    }
    let object = settings
        .as_object_mut()
        .context("the Claude Code settings are not a JSON object")?;
    object.insert(
        "statusLine".to_string(),
        serde_json::json!({ "type": "command", "command": command, "padding": 0 }),
    );
    write_settings(path, &settings, source.as_deref())
}

fn remove_from(path: &Path) -> Result<()> {
    let (mut settings, source) = load_settings_with_source(path)?;
    if !command_in(&settings).is_some_and(|command| is_bridge_command(&command)) {
        return Ok(());
    }
    if let Some(object) = settings.as_object_mut() {
        object.remove("statusLine");
    }
    write_settings(path, &settings, source.as_deref())
}

fn write_settings(
    path: &Path,
    settings: &serde_json::Value,
    original: Option<&[u8]>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create the Claude Code configuration directory")?;
    }
    let current = match std::fs::read(path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("re-read the Claude Code settings"),
    };
    anyhow::ensure!(
        current.as_deref() == original,
        "Claude Code settings changed while the status line was being updated; try again"
    );
    let content = serde_json::to_string_pretty(settings)?;
    // Claude Code reads this file continuously, so it is replaced whole rather than
    // truncated and rewritten in place.
    let staging = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&staging, format!("{content}\n")).context("write the Claude Code settings")?;
    std::fs::rename(&staging, path).map_err(|error| {
        let _ = std::fs::remove_file(&staging);
        error
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(now: i64) -> Reading {
        Reading {
            observed_at: now,
            five_hour: Some(Bucket { used_percentage: Some(23.5), resets_at: Some(now + 3_600) }),
            seven_day: Some(Bucket { used_percentage: Some(41.2), resets_at: Some(now + 86_400) }),
        }
    }

    #[test]
    fn both_reported_windows_map_onto_the_shared_quota_vocabulary() {
        let now = 1_800_000_000;
        let windows = windows_from(&reading(now), now).expect("valid reading");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "5-hour window");
        assert_eq!(windows[0].remaining_percent, Some(76.5));
        assert_eq!(windows[0].resets_at, Some(now + 3_600));
        assert_eq!(windows[1].label, "Weekly window");
        assert_eq!(windows[1].kind, LimitKind::Secondary);
    }

    #[test]
    fn a_window_that_has_already_restarted_is_dropped_rather_than_shown_expired() {
        let now = 1_800_000_000;
        let mut stale = reading(now);
        stale.five_hour = Some(Bucket { used_percentage: Some(90.0), resets_at: Some(now - 60) });
        let windows = windows_from(&stale, now).expect("remaining window is valid");
        assert_eq!(windows.len(), 1, "only the weekly window is still running");
        assert_eq!(windows[0].kind, LimitKind::Secondary);
    }

    #[test]
    fn a_reading_left_by_a_long_finished_session_is_treated_as_absent() {
        let now = 1_800_000_000;
        let old = Reading { observed_at: now - MAX_READING_AGE_SECS - 1, ..reading(now) };
        assert!(windows_from(&old, now).expect("old reading is ignored").is_empty());
    }

    #[test]
    fn malformed_known_bucket_is_schema_incompatible() {
        let now = 1_800_000_000;
        let malformed = Reading {
            observed_at: now,
            five_hour: Some(Bucket { used_percentage: None, resets_at: Some(now + 3_600) }),
            seven_day: None,
        };
        let error = windows_from(&malformed, now).expect_err("missing percentage must fail");
        assert!(error.to_string().contains("schema_incompatible"));
    }

    #[test]
    fn a_payload_without_rate_limits_still_produces_a_status_line() {
        assert_eq!(status_line(None, None, None), "QuotaStation");
        assert_eq!(status_line(Some("Opus"), None, None), "Opus");
    }

    #[test]
    fn the_status_line_reports_the_windows_it_stored() {
        let line = status_line(
            Some("Opus"),
            Some(Bucket { used_percentage: Some(23.5), resets_at: Some(1) }),
            Some(Bucket { used_percentage: Some(41.2), resets_at: Some(2) }),
        );
        assert_eq!(line, "Opus · 5h 24% used · 7d 41% used");
    }

    #[test]
    fn the_installed_command_is_recognised_however_the_path_is_quoted() {
        assert!(is_bridge_command("\"C:\\Program Files\\QuotaStation\\quotastation.exe\" --claude-statusline"));
        assert!(!is_bridge_command("npx ccusage statusline"));
    }

    fn scratch_settings(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("quotastation-{name}-settings.json"));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn installing_adds_the_status_line_and_leaves_every_other_setting_alone() {
        let path = scratch_settings("install");
        std::fs::write(&path, r#"{"env":{"A":"1"},"inputNeededNotifEnabled":true}"#).unwrap();
        install_into(&path, "\"q.exe\" --claude-statusline").expect("install the status line");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["type"], "command");
        assert_eq!(settings["statusLine"]["command"], "\"q.exe\" --claude-statusline");
        assert_eq!(settings["env"]["A"], "1", "unrelated settings survive the write");
        assert_eq!(settings["inputNeededNotifEnabled"], true);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.find("\"env\"").unwrap()
                < written.find("\"inputNeededNotifEnabled\"").unwrap()
        );

        remove_from(&path).expect("remove the status line");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(settings.get("statusLine").is_none());
        assert_eq!(settings["env"]["A"], "1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_status_line_belonging_to_something_else_is_never_replaced_or_removed() {
        let path = scratch_settings("foreign");
        let foreign = r#"{"statusLine":{"type":"command","command":"npx ccusage statusline"}}"#;
        std::fs::write(&path, foreign).unwrap();
        assert!(install_into(&path, "\"q.exe\" --claude-statusline").is_err());
        remove_from(&path).expect("removing leaves a foreign status line in place");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["command"], "npx ccusage statusline");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_settings_file_is_written_from_nothing() {
        let path = scratch_settings("absent");
        install_into(&path, "\"q.exe\" --claude-statusline").expect("install the status line");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["command"], "\"q.exe\" --claude-statusline");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_concurrent_settings_change_is_not_overwritten() {
        let path = scratch_settings("concurrent");
        std::fs::write(&path, r#"{"first":true}"#).unwrap();
        let (_, original) = load_settings_with_source(&path).expect("read original settings");
        std::fs::write(&path, r#"{"changed":true}"#).unwrap();
        let replacement = serde_json::json!({ "statusLine": { "command": "q" } });
        assert!(write_settings(&path, &replacement, original.as_deref()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"changed":true}"#);
        let _ = std::fs::remove_file(&path);
    }

    /// The payload Claude Code documents, read exactly as it arrives.
    #[test]
    fn the_documented_payload_is_read_as_two_windows() {
        let payload = r#"{
            "model": { "id": "claude-opus-5", "display_name": "Opus" },
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
                "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 }
            }
        }"#;
        let input: StatusLineInput = serde_json::from_str(payload).expect("read the payload");
        let limits = input.rate_limits.expect("the payload carries rate limits");
        assert_eq!(
            limits.five_hour,
            Some(Bucket { used_percentage: Some(23.5), resets_at: Some(1_738_425_600) })
        );
        assert_eq!(
            limits.seven_day,
            Some(Bucket { used_percentage: Some(41.2), resets_at: Some(1_738_857_600) })
        );
    }
}
