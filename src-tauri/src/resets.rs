//! Recognising when Codex restarted a quota window.
//!
//! Codex publishes a usage percentage and an expiry for each window. Both move on their
//! own: the percentage falls as older requests age out of a rolling window, and the
//! expiry creeps forward with them. A reset is the different event where the server
//! rebuilds the counter, and it is worth recording because it silently moves the expiry
//! days later than the one the client was shown.

use std::collections::BTreeMap;

use crate::domain::{LimitKind, LimitResetEvent, ResetClassification};

/// A window restarted more than this long before its published expiry did not restart on
/// the schedule Codex advertised. Two hours absorbs the polling interval and the drift
/// between the reported expiry and the instant the counter actually turned over.
pub const UNPLANNED_THRESHOLD_SECONDS: i64 = 2 * 60 * 60;

/// Usage has to fall from a meaningful level to effectively nothing. A partial fall is
/// the rolling window ageing out earlier requests, which is not a reset.
const BEFORE_MIN_USED_PERCENT: f64 = 5.0;
const AFTER_MAX_USED_PERCENT: f64 = 1.0;

/// The published expiry drifts forward continuously as the window ages, so only a jump
/// distinguishes a restart from that drift.
const MIN_FORWARD_SHIFT_SECONDS: i64 = 60;

/// A window that restarted between two observations is anchored at the first request
/// after the restart, so its anchor cannot predate the earlier observation. The slack
/// absorbs clock differences between the Codex server and this machine.
const ANCHOR_SLACK_SECONDS: i64 = 300;

#[derive(Debug, Clone, Copy)]
pub struct WindowObservation {
    pub observed_at: i64,
    pub kind: LimitKind,
    pub used_percent: f64,
    pub window_duration_mins: i64,
    pub resets_at: i64,
}

/// Compares one window against the previous reading of the same window. Every condition
/// has to hold: usage collapsed, the expiry jumped forward, and the new window is
/// anchored inside the gap between the two readings.
pub fn detect(previous: WindowObservation, current: WindowObservation) -> Option<LimitResetEvent> {
    if previous.window_duration_mins != current.window_duration_mins {
        return None;
    }
    if previous.used_percent < BEFORE_MIN_USED_PERCENT || current.used_percent > AFTER_MAX_USED_PERCENT {
        return None;
    }
    if current.resets_at - previous.resets_at < MIN_FORWARD_SHIFT_SECONDS {
        return None;
    }
    let anchored_at = current.resets_at - current.window_duration_mins * 60;
    if anchored_at < previous.observed_at - ANCHOR_SLACK_SECONDS {
        return None;
    }
    let early_by_seconds = previous.resets_at - anchored_at;
    Some(LimitResetEvent {
        window_kind: current.kind,
        window_label: current.kind.window_label(Some(current.window_duration_mins)),
        window_duration_mins: current.window_duration_mins,
        anchored_at,
        new_resets_at: current.resets_at,
        previous_resets_at: previous.resets_at,
        used_percent_before: previous.used_percent,
        early_by_seconds,
        classification: if early_by_seconds > UNPLANNED_THRESHOLD_SECONDS {
            ResetClassification::Unplanned
        } else {
            ResetClassification::Scheduled
        },
    })
}

/// Runs [`detect`] over a time-ordered stream of observations, keeping the previous
/// reading of each window kind. Codex has renamed its windows before — `primary` carried
/// the five-hour window and now carries the weekly one — so a change of duration only
/// reseeds the comparison instead of reporting a reset.
#[derive(Default)]
pub struct ResetTracker {
    previous: BTreeMap<&'static str, WindowObservation>,
}

impl ResetTracker {
    pub fn push(&mut self, observation: WindowObservation) -> Option<LimitResetEvent> {
        let key = match observation.kind {
            LimitKind::Primary => "primary",
            LimitKind::Secondary => "secondary",
        };
        let event = self.previous.get(key).and_then(|previous| detect(*previous, observation));
        self.previous.insert(key, observation);
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK_MINUTES: i64 = 10_080;
    const WEEK_SECONDS: i64 = WEEK_MINUTES * 60;

    fn observation(observed_at: i64, used_percent: f64, resets_at: i64) -> WindowObservation {
        WindowObservation {
            observed_at,
            kind: LimitKind::Primary,
            used_percent,
            window_duration_mins: WEEK_MINUTES,
            resets_at,
        }
    }

    #[test]
    fn a_window_restarted_long_before_its_published_expiry_is_unplanned() {
        // The weekly window still had days left and 52% of it was spent.
        let previous = observation(1_000_000, 52.0, 1_000_000 + 4 * 86_400);
        let current = observation(1_010_000, 0.0, 1_009_000 + WEEK_SECONDS);
        let event = detect(previous, current).expect("a collapse to zero is a reset");
        assert_eq!(event.classification, ResetClassification::Unplanned);
        assert_eq!(event.anchored_at, 1_009_000);
        assert_eq!(event.early_by_seconds, previous.resets_at - 1_009_000);
        assert_eq!(event.used_percent_before, 52.0);
    }

    #[test]
    fn a_window_restarting_at_its_published_expiry_is_scheduled() {
        let expiry = 1_000_000;
        let previous = observation(expiry - 600, 65.0, expiry);
        let current = observation(expiry + 600, 0.0, expiry + 300 + WEEK_SECONDS);
        let event = detect(previous, current).expect("an expiry is still a reset");
        assert_eq!(event.classification, ResetClassification::Scheduled);
    }

    #[test]
    fn a_rolling_window_ageing_out_earlier_requests_is_not_a_reset() {
        // Usage falls and the expiry drifts forward, but neither goes far enough.
        let previous = observation(1_000_000, 15.0, 1_000_000 + 3 * 86_400);
        let current = observation(1_020_000, 4.0, 1_000_000 + 3 * 86_400 + 20_000);
        assert!(detect(previous, current).is_none());
    }

    #[test]
    fn a_window_anchored_before_the_earlier_reading_is_not_a_reset() {
        // Zero usage against a window that started long ago describes a stale reading,
        // not a restart that happened between these two observations.
        let previous = observation(1_000_000, 40.0, 1_000_000 + 86_400);
        let current = observation(1_010_000, 0.0, 900_000 + WEEK_SECONDS);
        assert!(detect(previous, current).is_none());
    }

    #[test]
    fn a_change_of_window_duration_only_reseeds_the_comparison() {
        let mut tracker = ResetTracker::default();
        assert!(tracker.push(observation(1_000_000, 60.0, 1_000_000 + 86_400)).is_none());
        let renamed = WindowObservation {
            window_duration_mins: 300,
            ..observation(1_010_000, 0.0, 1_010_000 + 18_000)
        };
        assert!(tracker.push(renamed).is_none(), "the window Codex now reports is a different one");
    }
}
