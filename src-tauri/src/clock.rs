//! The application's own sense of time: the zone every hour and day bucket is keyed in and
//! every time is shown in, and how far this computer's clock is off.
//!
//! Instants are stored in UTC and need no zone. Only a bucket key — the local hour or day
//! a piece of usage belongs to — and a displayed time do, and both follow the zone chosen
//! in Settings, or the Windows zone when none is chosen. A computer whose Windows zone is
//! wrong can therefore still show the right times and exchange usage with the others.
//!
//! A wrong clock is a different fault, and no zone repairs it: countdowns run against it and
//! a reading dated by it no longer lines up with the server's reset times. Providers publish
//! no server time, so when the user opts in, the offset is measured against internet time
//! with one SNTP request — QuotaStation's only outbound request of its own, carrying no user
//! data — and [`now`] applies it.

use std::{
    net::UdpSocket,
    sync::{
        RwLock,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use jiff::{SignedDuration, Timestamp, civil::Date, tz::TimeZone};

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
    now().to_zoned(zone()).date()
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

/// How far this computer's clock is behind internet time, in milliseconds: add it to the
/// local clock for the corrected time. Zero until a check has measured it.
static OFFSET_MS: AtomicI64 = AtomicI64::new(0);

/// Moves on each time the check is switched off, so a measurement already in flight when it
/// was lands on nothing instead of restoring an offset the user has just turned off.
static SWITCHED_OFF: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// The offset a test reads, kept to its own thread for the same reason as the zone.
    static TEST_OFFSET_MS: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

/// The last check's outcome, for Diagnostics.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockCheck {
    /// When the offset was last measured.
    pub last_checked_at: Option<String>,
    /// Why the last attempt measured nothing, which leaves the previous offset in force.
    pub error: Option<String>,
}

static CHECK: RwLock<ClockCheck> = RwLock::new(ClockCheck { last_checked_at: None, error: None });

/// Standard SNTP on the port every time server answers.
const TIME_SERVER: &str = "time.windows.com:123";
/// UDP reports nothing when an answer never comes, so the wait is bounded.
const TIME_SERVER_TIMEOUT: Duration = Duration::from_secs(5);
/// Seconds from the NTP epoch, 1900, to the Unix one.
const NTP_UNIX_OFFSET: i64 = 2_208_988_800;
const NTP_PACKET_LEN: usize = 48;

pub fn offset_ms() -> i64 {
    #[cfg(test)]
    return TEST_OFFSET_MS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    OFFSET_MS.load(Ordering::Relaxed)
}

fn store_offset(offset_ms: i64) {
    #[cfg(test)]
    TEST_OFFSET_MS.with(|current| current.set(offset_ms));
    OFFSET_MS.store(offset_ms, Ordering::Relaxed);
}

/// The current time, corrected by the measured offset. This is what a reading is dated by
/// and what a server's reset time is compared against; a time the clients wrote into their
/// own logs is left as they wrote it, because the offset at that moment is unknown.
pub fn now() -> Timestamp {
    Timestamp::now()
        .checked_add(SignedDuration::from_millis(offset_ms()))
        .unwrap_or_else(|_| Timestamp::now())
}

pub fn last_check() -> ClockCheck {
    CHECK.read().map(|check| check.clone()).unwrap_or_default()
}

/// Measures the offset when the check is on, and forgets it when it is off. A failed
/// measurement leaves the previous offset in force and reports why.
pub async fn refresh_offset(enabled: bool) {
    refresh_offset_with(enabled, query_offset).await;
}

async fn refresh_offset_with(enabled: bool, query: fn() -> Result<i64>) {
    if !enabled {
        SWITCHED_OFF.fetch_add(1, Ordering::Relaxed);
        store_offset(0);
        if let Ok(mut check) = CHECK.write() {
            *check = ClockCheck::default();
        }
        return;
    }
    let switched_off = SWITCHED_OFF.load(Ordering::Relaxed);
    let measured = tokio::task::spawn_blocking(query)
        .await
        .map_err(anyhow::Error::from)
        .and_then(|result| result);
    if SWITCHED_OFF.load(Ordering::Relaxed) != switched_off {
        return;
    }
    let checked_at = Timestamp::now().to_string();
    match measured {
        Ok(offset) => {
            store_offset(offset);
            crate::log::write(format!("clock checked against internet time: offset {offset}ms"));
            if let Ok(mut check) = CHECK.write() {
                *check = ClockCheck { last_checked_at: Some(checked_at), error: None };
            }
        }
        Err(error) => {
            let message = crate::sanitize::sanitize_error(&error.to_string(), "Clock check failed");
            crate::log::write(format!("clock check failed: {message}"));
            if let Ok(mut check) = CHECK.write() {
                check.error = Some(message);
            }
        }
    }
}

/// One SNTP exchange: client mode, version 4, with this computer's time as the transmit
/// timestamp, which the server hands back so the answer can be matched to the request.
fn query_offset() -> Result<i64> {
    let socket = UdpSocket::bind("0.0.0.0:0").context("open a UDP socket")?;
    socket.set_read_timeout(Some(TIME_SERVER_TIMEOUT))?;
    socket.connect(TIME_SERVER).context("reach the time server")?;
    let mut request = [0u8; NTP_PACKET_LEN];
    request[0] = 0b00_100_011;
    let sent_at = Timestamp::now();
    let sent = to_ntp(sent_at);
    request[40..48].copy_from_slice(&sent);
    socket.send(&request).context("send the time request")?;
    let mut response = [0u8; 128];
    let length = socket.recv(&mut response).context("no answer from the time server")?;
    let received_at = Timestamp::now();
    parse_response(
        &response[..length],
        sent,
        sent_at.as_millisecond(),
        received_at.as_millisecond(),
    )
}

fn to_ntp(timestamp: Timestamp) -> [u8; 8] {
    let seconds = (timestamp.as_second() + NTP_UNIX_OFFSET) as u32;
    let fraction = ((i64::from(timestamp.subsec_nanosecond()) << 32) / 1_000_000_000) as u32;
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&seconds.to_be_bytes());
    bytes[4..].copy_from_slice(&fraction.to_be_bytes());
    bytes
}

/// An NTP timestamp as Unix milliseconds. The 32-bit seconds field wraps in 2036; the era
/// it belongs to is the one nearest this computer's own clock.
fn ntp_millis(bytes: &[u8], near_ms: i64) -> i64 {
    let seconds = i64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    let fraction = i64::from(u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
    let era = 1_i64 << 32;
    let near = near_ms.div_euclid(1_000) + NTP_UNIX_OFFSET;
    let seconds = seconds + era * ((near - seconds + era / 2).div_euclid(era));
    (seconds - NTP_UNIX_OFFSET) * 1_000 + ((fraction * 1_000 + (1 << 31)) >> 32)
}

/// The offset a server's answer implies: `((t2 − t1) + (t3 − t4)) / 2`, where t1 and t4
/// are when this computer sent and received and t2 and t3 when the server did.
fn parse_response(response: &[u8], sent: [u8; 8], t1: i64, t4: i64) -> Result<i64> {
    anyhow::ensure!(response.len() >= NTP_PACKET_LEN, "the time server's answer was truncated");
    let leap = response[0] >> 6;
    let mode = response[0] & 0b111;
    let stratum = response[1];
    anyhow::ensure!(mode == 4 && leap != 3, "the time server answered as an unsynchronised server");
    anyhow::ensure!((1..=15).contains(&stratum), "the time server declined to answer");
    anyhow::ensure!(response[24..32] == sent, "the time server answered a different request");
    let t2 = ntp_millis(&response[32..40], t1);
    let t3 = ntp_millis(&response[40..48], t1);
    Ok(((t2 - t1) + (t3 - t4)) / 2)
}

#[cfg(test)]
pub fn set_offset_for_test(offset_ms: i64) {
    store_offset(offset_ms);
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

    /// A server answer seven minutes behind this computer, taking 40 ms each way.
    fn answer(sent: [u8; 8], server_ms: i64) -> [u8; NTP_PACKET_LEN] {
        let mut response = [0u8; NTP_PACKET_LEN];
        response[0] = 0b00_100_100;
        response[1] = 2;
        response[24..32].copy_from_slice(&sent);
        let at = |ms: i64| to_ntp(Timestamp::from_millisecond(ms).unwrap());
        response[32..40].copy_from_slice(&at(server_ms));
        response[40..48].copy_from_slice(&at(server_ms + 2));
        response
    }

    #[test]
    fn a_recorded_answer_gives_the_offset_of_this_computers_clock() {
        let t1 = 1_800_000_000_000;
        let sent = to_ntp(Timestamp::from_millisecond(t1).unwrap());
        // The server received at t1 + 40 ms of true time, which is 7 minutes behind ours.
        let response = answer(sent, t1 + 40 - 420_000);
        let offset = parse_response(&response, sent, t1, t1 + 82).expect("a valid answer");
        assert_eq!(offset, -420_000, "this clock is seven minutes fast");
    }

    #[test]
    fn an_answer_to_another_request_is_refused() {
        let t1 = 1_800_000_000_000;
        let sent = to_ntp(Timestamp::from_millisecond(t1).unwrap());
        let other = to_ntp(Timestamp::from_millisecond(t1 - 1_000).unwrap());
        assert!(parse_response(&answer(other, t1), sent, t1, t1 + 80).is_err());
    }

    #[test]
    fn a_measured_offset_corrects_the_time_readings_are_dated_by() {
        set_offset_for_test(-420_000);
        let corrected = now().as_millisecond();
        let local = Timestamp::now().as_millisecond();
        set_offset_for_test(0);
        assert!((local - corrected - 420_000).abs() < 1_000);
    }

    /// Sends one real request to the time server. Ignored by default because it needs the
    /// network; run it with `cargo test time_server -- --ignored --nocapture` after
    /// changing the exchange.
    #[tokio::test]
    #[ignore = "sends a real SNTP request"]
    async fn the_time_server_answers_with_a_plausible_offset() {
        let offset = tokio::task::spawn_blocking(query_offset)
            .await
            .expect("the query ran")
            .expect("the time server answered");
        println!("offset {offset}ms");
        assert!(offset.abs() < 3_600_000);
    }

    #[tokio::test]
    async fn with_the_check_off_no_request_is_sent_and_the_offset_is_forgotten() {
        set_offset_for_test(90_000);
        refresh_offset_with(false, || panic!("no request may be sent with the check off")).await;
        assert_eq!(offset_ms(), 0);
    }

    #[tokio::test]
    async fn a_measurement_finishing_after_the_check_is_switched_off_is_discarded() {
        let slow_answer = || {
            std::thread::sleep(Duration::from_millis(200));
            Ok(90_000)
        };
        let switch_off = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            refresh_offset_with(false, || unreachable!()).await;
        };
        tokio::join!(refresh_offset_with(true, slow_answer), switch_off);
        assert_eq!(offset_ms(), 0);
    }
}
