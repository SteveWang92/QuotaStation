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

/// A material fall is required, but the first poll of a restarted window may already have
/// recorded work. Requiring it to remain below one percent permanently missed those resets.
const MIN_USED_PERCENT_DROP: f64 = 5.0;

/// The published expiry drifts forward continuously as the window ages, so only a jump
/// distinguishes a restart from that drift.
const MIN_FORWARD_SHIFT_SECONDS: i64 = 60;

/// A window that restarted between two observations is anchored at the first request
/// after the restart, so its anchor cannot predate the earlier observation. The slack
/// absorbs clock differences between the Codex server and this machine.
const ANCHOR_SLACK_SECONDS: i64 = 300;
pub const MAX_WINDOW_DURATION_MINS: i64 = 366 * 24 * 60;

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
/// anchored inside the gap between the two readings. The percentage range check also
/// rejects a non-finite reading, since no comparison against NaN succeeds.
///
/// The two readings must already describe the same window; callers pair them by duration,
/// which is what a window is, rather than by the slot Codex published it in.
pub fn detect(previous: WindowObservation, current: WindowObservation) -> Option<LimitResetEvent> {
    if current.observed_at < previous.observed_at
        || !(0.0..=100.0).contains(&previous.used_percent)
        || !(0.0..=100.0).contains(&current.used_percent)
        || !(1..=MAX_WINDOW_DURATION_MINS).contains(&current.window_duration_mins)
    {
        return None;
    }
    if previous.used_percent - current.used_percent < MIN_USED_PERCENT_DROP {
        return None;
    }
    if current.resets_at.checked_sub(previous.resets_at)? < MIN_FORWARD_SHIFT_SECONDS {
        return None;
    }
    let anchored_at = current
        .window_duration_mins
        .checked_mul(60)
        .and_then(|duration| current.resets_at.checked_sub(duration))?;
    if anchored_at < previous.observed_at.saturating_sub(ANCHOR_SLACK_SECONDS)
        || anchored_at > current.observed_at.saturating_add(ANCHOR_SLACK_SECONDS)
    {
        return None;
    }
    let early_by_seconds = previous.resets_at.checked_sub(anchored_at)?;
    Some(LimitResetEvent {
        window_kind: current.kind,
        window_label: current.kind.window_label(Some(current.window_duration_mins)),
        window_duration_mins: current.window_duration_mins,
        anchored_at,
        new_resets_at: current.resets_at,
        previous_resets_at: previous.resets_at,
        used_percent_before: previous.used_percent,
        // Detection compares two quota readings and knows nothing of the usage rows; the
        // total is attached when the stored event is read back.
        tokens_in_window: None,
        early_by_seconds,
        classification: if early_by_seconds > UNPLANNED_THRESHOLD_SECONDS {
            ResetClassification::Unplanned
        } else {
            ResetClassification::Scheduled
        },
    })
}

/// Runs [`detect`] over a time-ordered stream of observations, keeping the previous
/// reading of each window.
///
/// A window is identified by how long it runs, not by the slot it arrives in. Codex has
/// moved its windows between `primary` and `secondary` more than once — on 2026-08-25 the
/// weekly window moved out of `primary` and the five-hour window moved in — and pairing by
/// slot silently threw away the reading each restart had to be recognised against. A
/// duration that has never been seen simply starts a comparison of its own.
#[derive(Default)]
pub struct ResetTracker {
    previous: BTreeMap<i64, WindowObservation>,
}

impl ResetTracker {
    pub fn push(&mut self, observation: WindowObservation) -> Option<LimitResetEvent> {
        let key = observation.window_duration_mins;
        let event = self.previous.get(&key).and_then(|previous| detect(*previous, observation));
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
    fn a_restart_is_detected_when_the_first_new_sample_is_already_two_percent_used() {
        let previous = observation(1_000_000, 52.0, 1_000_000 + 4 * 86_400);
        let current = observation(1_010_000, 2.0, 1_009_000 + WEEK_SECONDS);
        assert!(detect(previous, current).is_some());
    }

    #[test]
    fn a_window_anchor_after_the_current_reading_is_rejected() {
        let previous = observation(1_000_000, 52.0, 1_000_000 + 4 * 86_400);
        let current = observation(1_010_000, 0.0, 1_020_000 + WEEK_SECONDS);
        assert!(detect(previous, current).is_none());
    }

    #[test]
    fn invalid_observation_values_are_rejected() {
        let previous = observation(1_000_000, 52.0, 1_000_000 + 4 * 86_400);
        assert!(detect(previous, observation(999_999, 0.0, 1_009_000 + WEEK_SECONDS)).is_none());
        assert!(
            detect(previous, observation(1_010_000, f64::NAN, 1_009_000 + WEEK_SECONDS)).is_none()
        );
        let invalid_duration = WindowObservation {
            window_duration_mins: 0,
            ..observation(1_010_000, 0.0, 1_009_000 + WEEK_SECONDS)
        };
        assert!(detect(previous, invalid_duration).is_none());
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
    fn a_window_duration_never_seen_before_only_seeds_a_comparison() {
        let mut tracker = ResetTracker::default();
        assert!(tracker.push(observation(1_000_000, 60.0, 1_000_000 + 86_400)).is_none());
        let five_hour = WindowObservation {
            window_duration_mins: 300,
            ..observation(1_010_000, 0.0, 1_010_000 + 18_000)
        };
        assert!(tracker.push(five_hour).is_none(), "a different window has nothing to compare to");
    }

    #[test]
    fn a_window_that_moved_to_the_other_slot_is_still_the_same_window() {
        // What Codex did on 2026-08-25: the weekly window left `primary`, the five-hour
        // window took its place, and everything restarted at zero.
        let mut tracker = ResetTracker::default();
        assert!(tracker.push(observation(1_000_000, 61.0, 1_000_000 + 4 * 86_400)).is_none());
        let five_hour = WindowObservation {
            kind: LimitKind::Primary,
            window_duration_mins: 300,
            ..observation(1_010_000, 0.0, 1_010_000 + 18_000)
        };
        assert!(tracker.push(five_hour).is_none());
        let weekly = WindowObservation {
            kind: LimitKind::Secondary,
            ..observation(1_010_000, 0.0, 1_010_000 + WEEK_SECONDS)
        };
        let event = tracker.push(weekly).expect("the weekly window restarted, wherever it arrived");
        assert_eq!(event.window_kind, LimitKind::Secondary);
        assert_eq!(event.used_percent_before, 61.0);
        assert_eq!(event.classification, ResetClassification::Unplanned);
    }
}
