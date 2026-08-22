//! The local hour buckets both provider parsers report their recent usage in.
//!
//! Hourly detail exists for one reason: a range of two or three days is unreadable as two
//! or three columns. It is therefore parsed and kept only as far back as such a range can
//! reach, and the daily rows remain the whole record beyond that.

use jiff::{Timestamp, tz::TimeZone};

use crate::domain::HOURLY_HISTORY_DAYS;

/// The local hour an instant fell in, as `YYYY-MM-DDTHH:00`.
pub fn hour_key(millis: i64, timezone: &TimeZone) -> Option<String> {
    let zoned = Timestamp::from_millisecond(millis).ok()?.to_zoned(timezone.clone());
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:00",
        zoned.year(),
        zoned.month(),
        zoned.day(),
        zoned.hour()
    ))
}

/// The oldest local date hourly rows are produced for, as `YYYY-MM-DD`.
///
/// The cutoff is a whole day rather than an exact instant so that a bucket is either
/// parsed in full or not at all, whatever time of day the parse runs at.
pub fn cutoff_date(timezone: &TimeZone) -> String {
    Timestamp::now()
        .to_zoned(timezone.clone())
        .date()
        .saturating_sub(jiff::Span::new().days(HOURLY_HISTORY_DAYS))
        .to_string()
}

/// Whether an hour key sits inside the hourly window, given that cutoff.
pub fn within_window(hour_start: &str, cutoff: &str) -> bool {
    hour_start.len() >= 10 && &hour_start[..10] >= cutoff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instant_becomes_the_local_hour_it_fell_in() {
        let timezone = TimeZone::get("Australia/Sydney").expect("a known zone");
        // 2026-08-21T03:30:00Z is 13:30 in Sydney, which is the 13:00 bucket.
        let millis = "2026-08-21T03:30:00Z".parse::<Timestamp>().unwrap().as_millisecond();
        assert_eq!(hour_key(millis, &timezone).as_deref(), Some("2026-08-21T13:00"));
    }

    #[test]
    fn the_window_is_read_off_the_date_part_alone() {
        assert!(within_window("2026-08-21T13:00", "2026-08-08"));
        assert!(within_window("2026-08-08T00:00", "2026-08-08"));
        assert!(!within_window("2026-08-07T23:00", "2026-08-08"));
    }
}
