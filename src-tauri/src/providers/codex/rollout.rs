//! Reading the rate-limit readings Codex already wrote to its own rollout logs.
//!
//! Codex records the server's answer alongside every token count, which reaches back far
//! further than QuotaStation's own samples and covers the stretches when it was not
//! running. Only the rate-limit fields are taken; conversation content is never read into
//! memory beyond the line being decoded, and never leaves this module.

use std::{env, fs::File, io::{BufRead, BufReader}, path::PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{domain::LimitKind, resets::WindowObservation};

const CODEX_HOME_OVERRIDE: &str = "CODEX_HOME";

/// Every rate-limit reading is attached to a token count, so lines without this marker
/// are skipped before the cost of decoding them is paid.
const RECORD_MARKER: &str = "token_count";

#[derive(Deserialize)]
struct RolloutLine {
    timestamp: String,
    payload: RolloutPayload,
}

#[derive(Deserialize)]
struct RolloutPayload {
    #[serde(default)]
    rate_limits: Option<RolloutRateLimits>,
}

#[derive(Deserialize)]
struct RolloutRateLimits {
    #[serde(default)]
    primary: Option<RolloutWindow>,
    #[serde(default)]
    secondary: Option<RolloutWindow>,
}

#[derive(Deserialize)]
struct RolloutWindow {
    used_percent: Option<f64>,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

/// Reads every rate-limit observation written on or after `since`, in time order.
///
/// `since` skips whole files by modification time, which keeps a routine startup from
/// re-reading months of transcripts. Passing `None` reads everything.
pub async fn read_observations(since: Option<i64>) -> Result<Vec<WindowObservation>> {
    tokio::task::spawn_blocking(move || read_observations_blocking(since))
        .await
        .context("Codex rollout scan stopped unexpectedly")?
}

fn read_observations_blocking(since: Option<i64>) -> Result<Vec<WindowObservation>> {
    let mut observations = Vec::new();
    for path in rollout_files(since)? {
        let Ok(file) = File::open(&path) else { continue };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if !line.contains(RECORD_MARKER) {
                continue;
            }
            let Ok(record) = serde_json::from_str::<RolloutLine>(&line) else { continue };
            let Some(limits) = record.payload.rate_limits else { continue };
            let Ok(observed) = record.timestamp.parse::<jiff::Timestamp>() else { continue };
            let observed_at = observed.as_second();
            for (window, kind) in [(limits.primary, LimitKind::Primary), (limits.secondary, LimitKind::Secondary)] {
                let Some(window) = window else { continue };
                let (Some(used_percent), Some(window_duration_mins), Some(resets_at)) =
                    (window.used_percent, window.window_minutes, window.resets_at)
                else {
                    continue;
                };
                observations.push(WindowObservation { observed_at, kind, used_percent, window_duration_mins, resets_at });
            }
        }
    }
    observations.sort_by_key(|observation| observation.observed_at);
    Ok(observations)
}

/// Live and archived sessions share one layout of dated directories, and both hold
/// readings worth recovering.
fn rollout_files(since: Option<i64>) -> Result<Vec<PathBuf>> {
    let home = codex_home()?;
    let mut files = Vec::new();
    for root in ["sessions", "archived_sessions"] {
        collect_rollout_files(&home.join(root), since, &mut files);
    }
    files.sort();
    Ok(files)
}

fn collect_rollout_files(directory: &std::path::Path, since: Option<i64>, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_files(&path, since, files);
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "jsonl") {
            continue;
        }
        if let Some(since) = since
            && !modified_since(&entry, since)
        {
            continue;
        }
        files.push(path);
    }
}

fn modified_since(entry: &std::fs::DirEntry, since: i64) -> bool {
    let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
        // An unreadable timestamp must not silently drop a file from the scan.
        return true;
    };
    match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() as i64 >= since,
        Err(_) => true,
    }
}

fn codex_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os(CODEX_HOME_OVERRIDE).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .context("locate the home directory that holds the Codex data folder")?;
    Ok(PathBuf::from(home).join(".codex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rollout_line_yields_one_observation_per_reported_window() {
        let line = r#"{"timestamp":"2026-08-10T12:53:51.548Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":45.0,"window_minutes":10080,"resets_at":1786835437},"secondary":null}}}"#;
        let record: RolloutLine = serde_json::from_str(line).expect("decode rollout line");
        let limits = record.payload.rate_limits.expect("rate limits present");
        assert_eq!(limits.primary.expect("primary window").used_percent, Some(45.0));
        assert!(limits.secondary.is_none(), "a null window carries no observation");
    }

    #[test]
    fn a_line_without_rate_limits_is_ignored() {
        let line = r#"{"timestamp":"2026-08-10T12:53:51.548Z","type":"event_msg","payload":{"type":"token_count"}}"#;
        let record: RolloutLine = serde_json::from_str(line).expect("decode rollout line");
        assert!(record.payload.rate_limits.is_none());
    }
}
