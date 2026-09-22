//! The application's own sense of time: the zone every hour and day bucket is keyed in and
//! every time is shown in.
//!
//! Instants are stored in UTC and need no zone. Only a bucket key — the local hour or day
//! a piece of usage belongs to — and a displayed time do, and both follow the zone chosen
//! in Settings, or the Windows zone when none is chosen. A computer whose Windows zone is
//! wrong can therefore still show the right times and exchange usage with the others.

use std::sync::RwLock;

use anyhow::{Context, Result};
use jiff::{Timestamp, civil::Date, tz::TimeZone};

/// The zone chosen in Settings; `None` follows Windows.
static CHOSEN: RwLock<Option<TimeZone>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    /// A zone one test sets for itself. The test runtime keeps a test on its own thread, so
    /// tests running beside it keep the zone they expect.
    static TEST_ZONE: std::cell::RefCell<Option<TimeZone>> = const { std::cell::RefCell::new(None) };
}

/// The zone every bucket and every displayed time follows.
pub fn zone() -> TimeZone {
    #[cfg(test)]
    if let Some(zone) = TEST_ZONE.with(|zone| zone.borrow().clone()) {
        return zone;
    }
    CHOSEN.read().ok().and_then(|chosen| chosen.clone()).unwrap_or_else(TimeZone::system)
}

/// The IANA name of [`zone`], which is what a parser is handed and what a shared file
/// records its buckets in.
pub fn zone_name() -> String {
    name_of(&zone())
}

/// The IANA name of the Windows zone, for the settings entry that follows it.
pub fn system_zone_name() -> String {
    name_of(&TimeZone::system())
}

fn name_of(zone: &TimeZone) -> String {
    zone.iana_name().unwrap_or("UTC").to_string()
}

/// A zone named in Settings. An unknown name is refused rather than quietly replaced by
/// the Windows zone, which is exactly the zone a person choosing one is getting away from.
pub fn resolve(name: &str) -> Result<TimeZone> {
    TimeZone::get(name).with_context(|| format!("{name} is not a known time zone"))
}

/// Makes a choice from Settings the zone in force.
pub fn choose(name: Option<&str>) -> Result<()> {
    let chosen = name.map(resolve).transpose()?;
    if let Ok(mut current) = CHOSEN.write() {
        *current = chosen;
    }
    Ok(())
}

/// Today's date in [`zone`].
pub fn today() -> Date {
    Timestamp::now().to_zoned(zone()).date()
}

/// The instant a local day begins in [`zone`]. A day that begins inside a gap — a DST
/// change at midnight — begins at the first instant after it.
pub fn day_start(date: Date) -> Result<i64> {
    Ok(date.to_zoned(zone())?.timestamp().as_second())
}

/// The local day an instant falls on, as `YYYY-MM-DD`.
pub fn day_key(epoch: i64) -> Option<String> {
    Some(Timestamp::from_second(epoch).ok()?.to_zoned(zone()).date().to_string())
}

/// The local hour an instant falls in, as `YYYY-MM-DDTHH:00`.
pub fn hour_key(epoch: i64) -> Option<String> {
    crate::providers::hours::hour_key(epoch.checked_mul(1_000)?, &zone())
}

#[cfg(test)]
pub fn set_for_test(name: Option<&str>) {
    let zone = name.map(|name| resolve(name).expect("a known test zone"));
    TEST_ZONE.with(|current| *current.borrow_mut() = zone);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_zone_is_refused() {
        assert!(resolve("Mars/Olympus_Mons").is_err());
        assert!(resolve("Australia/Sydney").is_ok());
    }

    #[test]
    fn bucket_keys_follow_the_chosen_zone_across_a_daylight_saving_change() {
        set_for_test(Some("Australia/Sydney"));
        // Sydney leaves daylight saving at 03:00 on 5 April 2026, repeating the 02:00 hour.
        let before = "2026-04-04T15:30:00Z".parse::<Timestamp>().unwrap().as_second();
        let after = "2026-04-04T16:30:00Z".parse::<Timestamp>().unwrap().as_second();
        assert_eq!(hour_key(before).as_deref(), Some("2026-04-05T02:00"));
        assert_eq!(hour_key(after).as_deref(), Some("2026-04-05T02:00"));
        assert_eq!(day_key(before).as_deref(), Some("2026-04-05"));
        let day = "2026-04-05".parse::<Date>().unwrap();
        let next = day.tomorrow().unwrap();
        assert_eq!(day_start(next).unwrap() - day_start(day).unwrap(), 25 * 3_600);
        set_for_test(None);
    }
}
