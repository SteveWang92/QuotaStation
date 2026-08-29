//! Writes the fictional database a `--demo` instance shows.
//!
//! Screenshots of QuotaStation are screenshots of somebody's account: the windows it draws
//! are quota readings and the charts under them are that person's working day. This fills a
//! separate database with a plausible fortnight that never happened, so a published picture
//! of the application shows the application rather than its author.
//!
//! The rows go in through the same [`Storage`] the application writes with — the live
//! readings through `save_live`, which is also what turns a restarted window into a reset
//! event, and the usage through `save_history`. Nothing here carries a copy of the schema,
//! so a migration that changes it changes this too, and a seeded database is always one the
//! current application can open.
//!
//! ```text
//! cargo run --example seed_demo --manifest-path src-tauri/Cargo.toml
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use jiff::{
    Span, Timestamp, Zoned,
    civil::{Date, Weekday},
};
use quotastation_lib::{
    demo,
    domain::{
        Freshness, HistoryDay, HistoryHour, HistorySnapshot, LimitKind, LimitWindow, LiveSnapshot,
        ModelUsage, ModelUsageRow, QuotaLevel, TokenUsage, WindowSource,
    },
    providers::ProviderKind,
    settings,
    storage::Storage,
};

/// The name the demonstration machine goes by. A device name reaches the interface wherever
/// usage is split by machine, so the seeded one is a workshop rather than somebody's laptop.
const DEVICE_NAME: &str = "Workshop";

/// How far back the daily usage chart can be dragged in the demonstration.
const HISTORY_DAYS: i64 = 45;
/// How far back hourly usage and quota readings go. The application keeps hourly rows for a
/// fortnight, so a longer demonstration would show a gap it cannot fill.
const OBSERVED_DAYS: i64 = 14;
/// The hours of the day the imaginary machine is worked.
const FIRST_HOUR: i8 = 8;
const LAST_HOUR: i8 = 22;

/// The hours a given day of the demonstration has work in.
///
/// Today only reaches as far as the clock has: a full day's usage recorded by breakfast is
/// the first thing a reader would notice. Seeded before the working day opens, that would
/// leave today empty and the newest column of every chart missing, so an early morning is
/// given the session that ran into it rather than nothing at all.
fn working_hours(today: bool, now: &Zoned) -> std::ops::RangeInclusive<i8> {
    if !today {
        return FIRST_HOUR..=LAST_HOUR;
    }
    let hour = now.hour();
    if hour < FIRST_HOUR { 0..=hour } else { FIRST_HOUR..=hour.min(LAST_HOUR) }
}

const FIVE_HOUR_MINS: i64 = 300;
const WEEK_MINS: i64 = 10_080;

/// One provider's imaginary subscription and working habits.
struct Profile {
    provider: ProviderKind,
    plan: &'static str,
    /// Model identifier, its share of the provider's tokens, and its price per million
    /// input, cache-read and output tokens. The prices only have to be the right shape: a
    /// cost estimate says where it came from, and this one came from here.
    models: &'static [(&'static str, f64, [f64; 3])],
    /// Tokens on an ordinary weekday, before the day's own variation.
    weekday_tokens: f64,
    /// Where the five-hour and weekly windows stand when the screenshot is taken.
    headline: (f64, f64),
}

const PROFILES: [Profile; 2] = [
    Profile {
        provider: ProviderKind::Codex,
        plan: "pro",
        models: &[
            ("gpt-5.1-codex", 0.82, [1.25, 0.125, 10.0]),
            ("gpt-5.1-codex-mini", 0.18, [0.25, 0.025, 2.0]),
        ],
        weekday_tokens: 4_100_000.0,
        headline: (68.0, 41.0),
    },
    Profile {
        provider: ProviderKind::Claude,
        plan: "max",
        models: &[
            ("claude-sonnet-5", 0.71, [3.0, 0.3, 15.0]),
            ("claude-opus-5", 0.29, [5.0, 0.5, 25.0]),
        ],
        weekday_tokens: 2_600_000.0,
        headline: (24.0, 79.0),
    },
];

#[tokio::main]
async fn main() -> Result<()> {
    let directory = application_data_directory()?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;

    // A seed replaces the demonstration, it does not add to it: yesterday's rows left behind
    // would put an unexplained gap in the middle of the charts.
    let database_path = directory.join(demo::DATABASE_FILE);
    remove_database(&database_path)?;

    let mut demo_settings = settings::load_default();
    demo_settings.device_id = Some("demo-workshop".to_string());
    demo_settings.device_name = Some(DEVICE_NAME.to_string());
    settings::save(&directory.join(demo::SETTINGS_FILE), &demo_settings)
        .map_err(|error| anyhow!("could not write the demonstration settings: {error}"))?;

    let storage = Storage::open(&database_path).await?;
    storage.record_local_device(DEVICE_NAME).await?;

    let now = Zoned::now();
    let timezone = now.time_zone().iana_name().unwrap_or("UTC").to_string();
    for profile in &PROFILES {
        seed_quota(&storage, profile, &now).await?;
        seed_usage(&storage, profile, &now, &timezone).await?;
    }

    println!("seeded {}", database_path.display());
    println!("start it with: quotastation.exe {}", demo::DEMO_ARG);
    Ok(())
}

/// `%APPDATA%\<identifier>`, the directory Tauri gives the application. The identifier is
/// read from the configuration rather than repeated here, so the seeder and the application
/// cannot come to disagree about where the demonstration database lives.
fn application_data_directory() -> Result<PathBuf> {
    let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
        .context("could not read tauri.conf.json")?;
    let identifier = config["identifier"].as_str().context("tauri.conf.json has no identifier")?;
    let roaming = std::env::var("APPDATA").context("APPDATA is not set")?;
    Ok(PathBuf::from(roaming).join(identifier))
}

/// Removes a previous demonstration database and the journal files beside it.
fn remove_database(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("could not remove {}", file.display()));
            }
        }
    }
    Ok(())
}

/// Replays a fortnight of quota readings, an hour apart through each working day.
///
/// The readings are what the quota chart draws, and they are also where the reset history
/// comes from: `save_live` compares each one against the reading before it, so a window that
/// turns over between two of them becomes a recorded restart exactly as it would in use. One
/// restart is deliberately early, because a demonstration that only ever shows scheduled
/// resets never shows what an unplanned one looks like.
async fn seed_quota(storage: &Storage, profile: &Profile, now: &Zoned) -> Result<()> {
    let source = profile.provider.authoritative_window_source();
    let five_hours = FIVE_HOUR_MINS * 60;
    let week = WEEK_MINS * 60;
    let mut rng = Rng::new(profile.provider.key());

    let observations = observation_times(now)?;
    let first = *observations.first().context("no observation times")?;
    let mut anchor = first - five_hours;
    let mut weekly_anchor = first - week / 2;
    let mut used = 34.0_f64;
    let mut weekly_used = 46.0_f64;
    // Codex is the provider whose restart runs early; one such event is enough to show the
    // classification, and two would read as a fault rather than a demonstration.
    let mut unplanned_pending = matches!(profile.provider, ProviderKind::Codex);

    for observed_at in observations {
        // Early by more than two hours is what the interface calls unplanned, so the
        // window has to be rebuilt with more than that still on it — and far enough into it
        // that the drop is one a reader can see.
        if unplanned_pending && anchor + five_hours - observed_at > 8_100 && used > 20.0 {
            // The counter was rebuilt while the window the provider had published still had
            // hours left on it, which is what the interface calls an unplanned reset.
            anchor = observed_at - 600;
            used = rng.between(5.0, 11.0);
            unplanned_pending = false;
        } else if observed_at >= anchor + five_hours {
            while observed_at >= anchor + five_hours {
                anchor += five_hours;
            }
            used = rng.between(3.0, 9.0);
        } else {
            used = (used + rng.between(4.0, 16.0)).min(97.0);
        }
        if observed_at >= weekly_anchor + week {
            while observed_at >= weekly_anchor + week {
                weekly_anchor += week;
            }
            weekly_used = rng.between(1.0, 4.0);
        } else {
            weekly_used = (weekly_used + rng.between(0.2, 1.4)).min(94.0);
        }

        let live = live_snapshot(
            profile,
            source,
            observed_at,
            (used, anchor + five_hours),
            (weekly_used, weekly_anchor + week),
        );
        storage.save_live(profile.provider, &live, &stamp(observed_at)?).await?;
    }

    // The reading the screenshot is taken of, chosen rather than wherever the replay
    // happened to end: one provider sits in its warning band and the other does not.
    let latest = now.timestamp().as_second() - 90;
    let mut current = anchor + five_hours;
    while current <= latest {
        current += five_hours;
    }
    let live = live_snapshot(
        profile,
        source,
        latest,
        (profile.headline.0, current),
        (profile.headline.1, weekly_anchor + week),
    );
    storage.save_live(profile.provider, &live, &stamp(latest)?).await?;
    Ok(())
}

fn live_snapshot(
    profile: &Profile,
    source: WindowSource,
    observed_at: i64,
    five_hour: (f64, i64),
    weekly: (f64, i64),
) -> LiveSnapshot {
    LiveSnapshot {
        plan_type: Some(profile.plan.to_string()),
        limits: vec![
            window(LimitKind::Primary, source, observed_at, FIVE_HOUR_MINS, five_hour),
            window(LimitKind::Secondary, source, observed_at, WEEK_MINS, weekly),
        ],
        earned_reset_count: None,
        earned_reset_expires_at: None,
    }
}

fn window(
    kind: LimitKind,
    source: WindowSource,
    observed_at: i64,
    duration_mins: i64,
    reading: (f64, i64),
) -> LimitWindow {
    LimitWindow {
        kind,
        label: kind.window_label(Some(duration_mins)),
        used_percent: Some((reading.0 * 10.0).round() / 10.0),
        window_duration_mins: Some(duration_mins),
        resets_at: Some(reading.1),
        source,
        observed_at,
        freshness: Freshness::Fresh,
        status_level: QuotaLevel::default(),
    }
}

/// Every hour a reading is taken, oldest first.
fn observation_times(now: &Zoned) -> Result<Vec<i64>> {
    let mut times = Vec::new();
    let today = now.date();
    for day_offset in (0..OBSERVED_DAYS).rev() {
        let date = today.saturating_sub(Span::new().days(day_offset));
        for hour in working_hours(day_offset == 0, now) {
            let at = date.at(hour, 0, 0, 0).to_zoned(now.time_zone().clone())?;
            if at.timestamp() < now.timestamp() {
                times.push(at.timestamp().as_second());
            }
        }
    }
    Ok(times)
}

/// Writes the usage the charts and tables are drawn from: every day of the range daily, and
/// the fortnight the hourly view reaches back over hour by hour as well.
async fn seed_usage(
    storage: &Storage,
    profile: &Profile,
    now: &Zoned,
    timezone: &str,
) -> Result<()> {
    let mut rng = Rng::new(profile.provider.display_name());
    let today = now.date();
    let mut days = Vec::new();
    let mut hours = Vec::new();

    for day_offset in (0..HISTORY_DAYS).rev() {
        let date = today.saturating_sub(Span::new().days(day_offset));
        let weekend = matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday);
        let mut tokens = profile.weekday_tokens * rng.between(0.55, 1.45);
        if weekend {
            tokens *= 0.28;
        }
        if day_offset == 0 {
            let hours = working_hours(true, now);
            let span = f64::from(hours.end() - hours.start() + 1);
            tokens *= span / f64::from(LAST_HOUR - FIRST_HOUR + 1);
        }
        if tokens < 10_000.0 {
            continue;
        }

        if day_offset >= OBSERVED_DAYS {
            days.push(history_day(date, model_rows(profile, tokens, &mut rng)));
            continue;
        }
        let mut day_rows: Vec<ModelUsageRow> = Vec::new();
        for hour in working_hours(day_offset == 0, now) {
            // Not every hour of a working day is spent in a coding session.
            let intensity = rng.between(0.0, 1.0);
            if intensity < 0.28 {
                continue;
            }
            let rows = model_rows(profile, tokens / 7.0 * intensity * 2.0, &mut rng);
            if rows.is_empty() {
                continue;
            }
            merge_rows(&mut day_rows, &rows);
            hours
                .push(HistoryHour { hour_start: format!("{date}T{hour:02}:00"), model_rows: rows });
        }
        if !day_rows.is_empty() {
            days.push(history_day(date, day_rows));
        }
    }

    let history = HistorySnapshot { days, hours };
    let observed_at = stamp(now.timestamp().as_second())?;
    storage.save_history(profile.provider, &history, timezone, &observed_at).await
}

/// Splits a bucket's tokens across the provider's models and prices each share.
fn model_rows(profile: &Profile, tokens: f64, rng: &mut Rng) -> Vec<ModelUsageRow> {
    let mut rows = Vec::new();
    for (model, share, prices) in profile.models {
        let total = tokens * share * rng.between(0.8, 1.2);
        if total < 2_000.0 {
            continue;
        }
        // A coding session re-reads far more context than it writes, and the cache is what
        // keeps that affordable; the split is what makes the cost estimate look like one.
        let cache_read = total * 0.76;
        let input = total * 0.13;
        let output = total * 0.08;
        let reasoning = total * 0.03;
        let cost = (input * prices[0] + cache_read * prices[1] + output * prices[2]) / 1_000_000.0;
        rows.push(ModelUsageRow {
            model: (*model).to_string(),
            input: input as u64,
            cache_read: cache_read as u64,
            output: output as u64,
            reasoning: reasoning as u64,
            total: total as u64,
            cost_usd: (cost * 10_000.0).round() / 10_000.0,
        });
    }
    rows
}

/// Adds an hour's rows into the day they belong to, so the daily and hourly views of the
/// demonstration agree the way a single parse of real sessions would make them agree.
fn merge_rows(into: &mut Vec<ModelUsageRow>, rows: &[ModelUsageRow]) {
    for row in rows {
        match into.iter_mut().find(|existing| existing.model == row.model) {
            Some(existing) => {
                existing.input += row.input;
                existing.cache_read += row.cache_read;
                existing.output += row.output;
                existing.reasoning += row.reasoning;
                existing.total += row.total;
                existing.cost_usd += row.cost_usd;
            }
            None => into.push(row.clone()),
        }
    }
}

fn history_day(date: Date, model_rows: Vec<ModelUsageRow>) -> HistoryDay {
    let usage = model_rows.iter().fold(TokenUsage::default(), |mut usage, row| {
        usage.input += row.input;
        usage.cache_read += row.cache_read;
        usage.output += row.output;
        usage.reasoning += row.reasoning;
        usage.total += row.total;
        usage
    });
    let total = usage.total.max(1) as f64;
    HistoryDay {
        date: date.to_string(),
        models: model_rows
            .iter()
            .map(|row| ModelUsage {
                model: row.model.clone(),
                tokens: row.total,
                percent: row.total as f64 / total * 100.0,
            })
            .collect(),
        cost_usd: model_rows.iter().map(|row| row.cost_usd).sum(),
        usage,
        model_rows,
    }
}

fn stamp(seconds: i64) -> Result<String> {
    Ok(Timestamp::from_second(seconds)?.to_string())
}

/// A deterministic generator, so the demonstration is the same one every time it is seeded
/// and a screenshot can be retaken to match an older one.
struct Rng(u64);

impl Rng {
    fn new(seed: &str) -> Self {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for byte in seed.as_bytes() {
            state = (state ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
        Self(state)
    }

    fn between(&mut self, low: f64, high: f64) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        low + (value >> 11) as f64 / (1_u64 << 53) as f64 * (high - low)
    }
}
