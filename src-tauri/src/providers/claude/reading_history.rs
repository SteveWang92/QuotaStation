//! The quota readings the status-line bridge has seen, kept so restarts that happened while
//! QuotaStation was closed can still be recognised.
//!
//! Claude Code runs the bridge on every status-line render whether QuotaStation is running
//! or not, but the bridge's reading file holds only the latest reading. A window that
//! restarted twice while the application was closed would leave one jump in expiry behind,
//! and the restart between would be lost. This record keeps enough of every window to
//! replay each restart: the first and the last reading of each run of one expiry, which is
//! the pair a restart is recognised between.
//!
//! It is Claude's counterpart of Codex's rollout logs, and the startup backfill replays it
//! the same way. It holds the quota vocabulary alone — no session, path or prompt.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::domain::{LimitKind, LimitWindow};
use crate::resets::WindowObservation;

use super::FIVE_HOUR_WINDOW_MINS;

const HISTORY_FILE: &str = "claude-status-line-history.json";

/// The version of the file this build writes. A reader skips a newer file rather than
/// replaying readings it may misread.
const FORMAT_VERSION: u32 = 1;

/// Readings older than this are dropped. It bounds the file, and a restart further back than
/// this, missed live and never replayed, is not worth the file growing for.
const KEPT_SECONDS: i64 = 35 * 24 * 60 * 60;

/// An unchanged reading is written no more often than this. The last reading of an expiry
/// dates the start of the pair its restart is recognised between, so it has to move on while
/// nothing else does, but not on every render.
const REFRESH_SECONDS: i64 = 10 * 60;

/// Expiries closer than this are one run of a window: the server rounds them to the second,
/// and a restart moves one by hours.
const SAME_EXPIRY_SECONDS: i64 = 60;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct History {
    format_version: u32,
    readings: Vec<Reading>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Reading {
    observed_at: i64,
    window_duration_mins: i64,
    used_percent: f64,
    resets_at: i64,
}

fn history_path() -> Option<PathBuf> {
    super::statusline::app_data_dir().map(|dir| dir.join(HISTORY_FILE))
}

/// Adds the bridge's latest windows to the record. Called by the bridge with the windows it
/// has just stored; a failure costs one reading of a record the next render adds to again.
pub(super) fn record(windows: &[LimitWindow], now: i64) -> Result<()> {
    let path = history_path().context("resolve the application data directory")?;
    let mut history = match load(&path)? {
        Some(history) if history.format_version > FORMAT_VERSION => return Ok(()),
        Some(history) => history,
        None => History::default(),
    };
    if add(&mut history, windows, now) {
        history.format_version = FORMAT_VERSION;
        // Several Claude Code sessions render at once; a reader must never see half a file.
        // Two renders racing lose one reading, which the next render restores.
        crate::fs_atomic::write(&path, serde_json::to_vec(&history)?)
            .context("store the status-line reading history")?;
    }
    Ok(())
}

/// Folds the windows into the record and reports whether anything changed.
fn add(history: &mut History, windows: &[LimitWindow], now: i64) -> bool {
    let before = history.readings.len();
    history.readings.retain(|reading| now - reading.observed_at <= KEPT_SECONDS);
    let mut changed = history.readings.len() != before;
    for window in windows {
        let (Some(used_percent), Some(window_duration_mins), Some(resets_at)) =
            (window.used_percent, window.window_duration_mins, window.resets_at)
        else {
            continue;
        };
        let reading = Reading {
            observed_at: window.observed_at,
            window_duration_mins,
            used_percent,
            resets_at,
        };
        let run: Vec<usize> = history
            .readings
            .iter()
            .enumerate()
            .rev()
            .take_while(|(_, kept)| {
                kept.window_duration_mins != window_duration_mins
                    || (kept.resets_at - resets_at).abs() < SAME_EXPIRY_SECONDS
            })
            .filter(|(_, kept)| kept.window_duration_mins == window_duration_mins)
            .map(|(index, _)| index)
            .collect();
        match run.as_slice() {
            // The run's latest reading moves on when the share changed or it has aged; its
            // first stays, because it dates the restart that began the run.
            [last, _, ..] => {
                let kept = history.readings[*last];
                if kept.used_percent != used_percent
                    || reading.observed_at - kept.observed_at >= REFRESH_SECONDS
                {
                    history.readings.remove(*last);
                    history.readings.push(reading);
                    changed = true;
                }
            }
            [first] => {
                let kept = history.readings[*first];
                if kept.used_percent != used_percent
                    || reading.observed_at - kept.observed_at >= REFRESH_SECONDS
                {
                    history.readings.push(reading);
                    changed = true;
                }
            }
            [] => {
                history.readings.push(reading);
                changed = true;
            }
        }
    }
    changed
}

fn load(path: &std::path::Path) -> Result<Option<History>> {
    match std::fs::read(path) {
        Ok(content) => serde_json::from_slice(&content)
            .map(Some)
            .context("decode the status-line reading history"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("read the status-line reading history"),
    }
}

/// Every kept reading observed on or after `since`, oldest first, for the restart backfill.
pub fn read_observations(since: Option<i64>) -> Result<Vec<WindowObservation>> {
    let Some(path) = history_path() else { return Ok(Vec::new()) };
    let Some(history) = load(&path)? else { return Ok(Vec::new()) };
    anyhow::ensure!(
        history.format_version <= FORMAT_VERSION,
        "the status-line reading history was written by a newer QuotaStation"
    );
    Ok(observations(&history, since))
}

fn observations(history: &History, since: Option<i64>) -> Vec<WindowObservation> {
    let mut observations: Vec<WindowObservation> = history
        .readings
        .iter()
        .filter(|reading| since.is_none_or(|since| reading.observed_at >= since))
        .map(|reading| WindowObservation {
            observed_at: reading.observed_at,
            kind: if reading.window_duration_mins == FIVE_HOUR_WINDOW_MINS {
                LimitKind::Primary
            } else {
                LimitKind::Secondary
            },
            used_percent: reading.used_percent,
            window_duration_mins: reading.window_duration_mins,
            resets_at: reading.resets_at,
        })
        .collect();
    observations.sort_by_key(|observation| observation.observed_at);
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Freshness, PaceLevel, QuotaLevel, WindowSource};
    use crate::resets::ResetTracker;

    const HOUR: i64 = 3_600;

    fn window(observed_at: i64, used_percent: f64, resets_at: i64) -> LimitWindow {
        LimitWindow {
            kind: LimitKind::Primary,
            label: "5-hour".to_string(),
            used_percent: Some(used_percent),
            window_duration_mins: Some(FIVE_HOUR_WINDOW_MINS),
            resets_at: Some(resets_at),
            source: WindowSource::StatusLine,
            observed_at,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
            pace: PaceLevel::OnTrack,
        }
    }

    #[test]
    fn two_restarts_while_the_application_was_closed_are_both_replayed() {
        let start = 1_800_000_000;
        let mut history = History::default();
        // Three five-hour windows, each read several times by the bridge alone.
        for (window_start, spent) in
            [(start, 60.0), (start + 5 * HOUR, 80.0), (start + 10 * HOUR, 40.0)]
        {
            let resets_at = window_start + 5 * HOUR;
            for step in 0..=4 {
                let observed_at = window_start + 60 + step * HOUR;
                let used = spent * step as f64 / 4.0;
                add(&mut history, &[window(observed_at, used, resets_at)], observed_at);
            }
        }
        assert_eq!(history.readings.len(), 6, "each run keeps its first and its last reading");

        let mut tracker = ResetTracker::default();
        let restarts = observations(&history, None)
            .into_iter()
            .filter_map(|observation| tracker.push(observation))
            .count();
        assert_eq!(restarts, 2);
    }

    #[test]
    fn an_unchanged_reading_is_not_written_again_until_it_has_aged() {
        let resets_at = 1_800_018_000;
        let mut history = History::default();
        assert!(add(&mut history, &[window(1_800_000_000, 10.0, resets_at)], 1_800_000_000));
        assert!(!add(&mut history, &[window(1_800_000_060, 10.0, resets_at)], 1_800_000_060));
        assert!(add(&mut history, &[window(1_800_000_700, 10.0, resets_at)], 1_800_000_700));
    }

    #[test]
    fn readings_older_than_the_kept_span_are_dropped() {
        let mut history = History::default();
        add(&mut history, &[window(1_000, 10.0, 19_000)], 1_000);
        add(
            &mut history,
            &[window(1_000 + KEPT_SECONDS + 1, 5.0, 99_000_000)],
            1_000 + KEPT_SECONDS + 1,
        );
        assert_eq!(history.readings.len(), 1);
        assert_eq!(history.readings[0].used_percent, 5.0);
    }
}
