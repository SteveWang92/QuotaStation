//! Claude quota windows recovered from the local session logs.
//!
//! This is the default source. Claude Code runs a rolling five-hour session window that
//! starts with the first request after the previous one ended, and its logs record every
//! request, so the window currently running and the moment it ends can both be recovered
//! locally without presenting a credential to anything. What the logs cannot give is how
//! much of the window has been consumed: Anthropic publishes no allowance in them, and
//! the percentage therefore stays unknown unless the usage endpoint fills it in.

use anyhow::{Context, Result, bail};
use ccusage_adapter_claude::load_entries;
use ccusage_core::cli::SharedArgs;

use crate::domain::{Freshness, LimitKind, LimitWindow, LiveSnapshot, QuotaLevel, WindowSource};

use super::FIVE_HOUR_WINDOW_MINS as WINDOW_MINS;

const WINDOW_MS: i64 = WINDOW_MINS * 60 * 1_000;
const HOUR_MS: i64 = 60 * 60 * 1_000;

const NO_LOCAL_HISTORY: &str = "No Claude Code session history was found on this machine. Sign in to Claude Code and \
     send a request, then refresh.";

pub async fn read_live(plan_type: Option<String>) -> Result<LiveSnapshot> {
    tokio::task::spawn_blocking(move || read_live_blocking(plan_type))
        .await
        .context("Claude history parser stopped unexpectedly")?
}

fn read_live_blocking(plan_type: Option<String>) -> Result<LiveSnapshot> {
    let entries = load_entries(&SharedArgs { json: true, ..SharedArgs::default() }, None)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    if entries.is_empty() {
        bail!("{NO_LOCAL_HISTORY}");
    }
    let now_ms = jiff::Timestamp::now().as_millisecond();
    let mut timestamps: Vec<i64> =
        entries.iter().map(|entry| entry.timestamp.as_millis()).collect();
    let observed_at = timestamps.iter().copied().max().unwrap().div_euclid(1_000);
    // ccusage exposes usage_limit_reset_time without the limit bucket that produced it.
    // Until that field's window semantics can be verified, assigning it to the five-hour
    // window would turn an ambiguous log value into a confident but potentially wrong timer.
    let resets_at_ms = current_window_end(&mut timestamps, now_ms);

    Ok(LiveSnapshot {
        plan_type,
        limits: vec![LimitWindow {
            kind: LimitKind::Primary,
            label: LimitKind::Primary.window_label(Some(WINDOW_MINS)),
            // The logs record what was spent, never what the allowance was.
            used_percent: None,
            window_duration_mins: Some(WINDOW_MINS),
            resets_at: resets_at_ms.map(|value| value.div_euclid(1_000)),
            source: WindowSource::SessionLog,
            observed_at,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
        }],
        // Claude grants no reset inventory of the kind Codex publishes.
        earned_reset_count: None,
    })
}

/// When the session window covering `now_ms` ends, or `None` when the last window already
/// closed and the next one starts with a request that has not happened yet.
///
/// A window opens at the request that starts it, rounded down to the hour, which is how
/// Claude itself reports session starts. It stays open five hours, and a gap longer than
/// that means the window closed unused and the next request opened a new one.
fn current_window_end(timestamps: &mut [i64], now_ms: i64) -> Option<i64> {
    timestamps.sort_unstable();
    let mut window_start = None;
    for &timestamp in timestamps.iter() {
        let starts_new_window = window_start.is_none_or(|start| timestamp - start >= WINDOW_MS);
        if starts_new_window {
            window_start = Some(timestamp.div_euclid(HOUR_MS) * HOUR_MS);
        }
    }
    let end = window_start? + WINDOW_MS;
    (end > now_ms).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = HOUR_MS;

    #[test]
    fn a_window_ends_five_hours_after_the_hour_its_first_request_fell_in() {
        let start = 100 * HOUR + 25 * 60 * 1_000;
        let mut timestamps = vec![start, start + HOUR];
        assert_eq!(
            current_window_end(&mut timestamps, start + 2 * HOUR),
            Some(100 * HOUR + WINDOW_MS)
        );
    }

    #[test]
    fn a_request_after_the_window_closed_opens_the_next_one() {
        let first = 100 * HOUR;
        let later = first + 9 * HOUR;
        let mut timestamps = vec![first, later];
        assert_eq!(current_window_end(&mut timestamps, later + HOUR), Some(109 * HOUR + WINDOW_MS));
    }

    #[test]
    fn a_closed_window_reports_no_reset_because_the_next_one_has_not_started() {
        let mut timestamps = vec![100 * HOUR];
        assert_eq!(current_window_end(&mut timestamps, 110 * HOUR), None);
    }

    /// Run with `cargo test claude_session_window -- --ignored --nocapture` to see what
    /// this machine's own logs currently describe.
    #[tokio::test]
    #[ignore = "requires local Claude Code session logs"]
    async fn claude_session_window_is_recovered_from_local_logs() {
        let live = read_live(None).await.expect("read the session window");
        for limit in &live.limits {
            println!("{} resets_at {:?}", limit.label, limit.resets_at);
        }
        assert_eq!(live.limits.len(), 1, "the session window is the only one the logs give");
    }

    #[test]
    fn requests_out_of_order_describe_the_same_window() {
        let base = 100 * HOUR;
        let mut ordered = vec![base, base + HOUR, base + 2 * HOUR];
        let mut shuffled = vec![base + 2 * HOUR, base, base + HOUR];
        let now = base + 3 * HOUR;
        assert_eq!(current_window_end(&mut ordered, now), current_window_end(&mut shuffled, now));
    }
}
