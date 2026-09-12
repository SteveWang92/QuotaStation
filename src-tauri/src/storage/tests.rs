use super::test_support::{TempDatabase, open_storage};
use super::*;
use crate::domain::{
    DeviceUsageRow, HistoryDay, HistoryHour, HistorySnapshot, LimitWindow, LiveSnapshot,
    ModelUsageRow, ResetClassification, SessionCost,
};
use crate::resets::WindowObservation;

const CODEX: ProviderKind = ProviderKind::Codex;

#[tokio::test]
async fn opens_database_in_unicode_directory_with_spaces() {
    let database = TempDatabase::in_unicode_directory();
    let storage = Storage::open(&database.path).await.expect("open unicode database path");
    drop(storage);
    assert!(database.path.exists());
}

fn day(date: &str, model: &str, total: u64) -> HistoryDay {
    HistoryDay {
        date: date.to_string(),
        usage: TokenUsage {
            input: total / 2,
            cache_read: 0,
            output: total / 2,
            reasoning: 0,
            total,
        },
        models: vec![ModelUsage { model: model.to_string(), tokens: total, percent: 100.0 }],
        cost_usd: 1.5,
        model_rows: vec![ModelUsageRow {
            model: model.to_string(),
            input: total / 2,
            cache_read: 0,
            output: total / 2,
            reasoning: 0,
            total,
            cost_usd: 1.5,
        }],
    }
}

fn hour(hour_start: &str, model: &str, total: u64) -> HistoryHour {
    HistoryHour {
        hour_start: hour_start.to_string(),
        model_rows: vec![ModelUsageRow {
            model: model.to_string(),
            input: total / 2,
            cache_read: 0,
            output: total / 2,
            reasoning: 0,
            total,
            cost_usd: 0.25,
        }],
    }
}

fn device_row(provider: &str, bucket: &str, model: &str, total: u64) -> DeviceUsageRow {
    DeviceUsageRow {
        provider: provider.to_string(),
        bucket: bucket.to_string(),
        model: model.to_string(),
        service_tier: "mixed".to_string(),
        input: total / 2,
        cache_read: 0,
        output: total / 2,
        reasoning: 0,
        total,
        cost_usd: Some(1.0),
    }
}

#[tokio::test]
async fn an_hourly_range_is_read_back_one_point_per_recorded_hour() {
    let (storage, _database) = open_storage().await;
    let history = HistorySnapshot {
        days: vec![day("2026-08-20", "gpt-5-codex", 400)],
        hours: vec![
            hour("2026-08-20T09:00", "gpt-5-codex", 300),
            hour("2026-08-20T14:00", "gpt-5-codex", 100),
        ],
    };
    storage
        .save_history(CODEX, &history, "Australia/Sydney", "2026-08-20T15:00:00Z")
        .await
        .expect("save history");

    let hours = storage
        .load_usage_hours(Some(CODEX), None, "2026-08-20", "2026-08-20")
        .await
        .expect("read the hourly range");
    assert_eq!(
        hours.hours.iter().map(|point| point.hour_start.as_str()).collect::<Vec<_>>(),
        ["2026-08-20T09:00", "2026-08-20T14:00"],
        "an hour nothing ran in is an absence rather than a zero"
    );
    assert_eq!(hours.hours[0].usage.total, 300);
    assert_eq!(hours.hours[1].models[0].model, "gpt-5-codex");
}

#[tokio::test]
async fn a_rolling_window_counts_only_the_hours_inside_it() {
    let (storage, _database) = open_storage().await;
    let history = HistorySnapshot {
        days: vec![day("2026-08-19", "gpt-5-codex", 100), day("2026-08-20", "gpt-5-codex", 700)],
        hours: vec![
            // Before the window opens: on the 19th, but earlier than 14:00.
            hour("2026-08-19T09:00", "gpt-5-codex", 100),
            hour("2026-08-19T18:00", "gpt-5-codex", 300),
            hour("2026-08-20T10:00", "gpt-5-codex", 400),
        ],
    };
    storage
        .save_history(CODEX, &history, "Australia/Sydney", "2026-08-20T15:00:00Z")
        .await
        .expect("save history");

    let window = storage
        .load_usage_window(Some(CODEX), None, "2026-08-19T14:00", "2026-08-20T13:00")
        .await
        .expect("read the rolling window");
    assert_eq!(window.range.usage.total, 700, "the hour before the window opened is outside it");
    assert_eq!(
        window.hours.hours.iter().map(|point| point.hour_start.as_str()).collect::<Vec<_>>(),
        ["2026-08-19T18:00", "2026-08-20T10:00"]
    );
    // Both days the window touches are partial, and each day row carries only the hours
    // of it that fell inside.
    assert_eq!(
        window
            .range
            .days
            .iter()
            .map(|point| (point.date.as_str(), point.usage.total))
            .collect::<Vec<_>>(),
        [("2026-08-19", 300), ("2026-08-20", 400)]
    );
}

#[tokio::test]
async fn re_parsing_an_hour_replaces_it_rather_than_adding_to_it() {
    let (storage, _database) = open_storage().await;
    for total in [300, 500] {
        let history = HistorySnapshot {
            days: vec![day("2026-08-20", "gpt-5-codex", total)],
            hours: vec![hour("2026-08-20T09:00", "gpt-5-codex", total)],
        };
        storage
            .save_history(CODEX, &history, "Australia/Sydney", "2026-08-20T15:00:00Z")
            .await
            .expect("save history");
    }
    let hours = storage
        .load_usage_hours(Some(CODEX), None, "2026-08-20", "2026-08-20")
        .await
        .expect("read the hourly range");
    assert_eq!(hours.hours.len(), 1);
    assert_eq!(hours.hours[0].usage.total, 500);
}

#[tokio::test]
async fn retention_drops_hourly_usage_once_it_leaves_the_window() {
    let (storage, _database) = open_storage().await;
    let history = HistorySnapshot {
        days: vec![day("2026-08-20", "gpt-5-codex", 400)],
        hours: vec![
            hour("2026-06-01T09:00", "gpt-5-codex", 100),
            hour("2026-08-20T09:00", "gpt-5-codex", 300),
        ],
    };
    storage
        .save_history(CODEX, &history, "Australia/Sydney", "2026-08-20T15:00:00Z")
        .await
        .expect("save history");

    storage.run_retention_at("2026-08-20T15:00:00Z").await.expect("run retention");

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT hour_start FROM hourly_usage ORDER BY hour_start")
            .fetch_all(&storage.pool)
            .await
            .expect("read the surviving hours");
    assert_eq!(remaining, ["2026-08-20T09:00"]);
    let days = storage
        .load_usage_range(Some(CODEX), None, "2026-06-01", "2026-08-20")
        .await
        .expect("read the daily range");
    assert_eq!(days.days.len(), 1, "the daily rows are untouched by the hourly cutoff");
}

fn session(
    session_id: &str,
    started_at: &str,
    reported: Option<f64>,
    computed: f64,
) -> SessionCost {
    SessionCost {
        session_id: session_id.to_string(),
        session_started_at: started_at.to_string(),
        duration_ms: 900_000,
        computed_cost_usd: computed,
        independent: true,
        reported_cost_usd: reported,
        reported_complete: reported.map(|_| true),
        api_duration_ms: reported.map(|_| 300_000),
        lines_added: reported.map(|_| 40),
        lines_removed: reported.map(|_| 5),
        usage: TokenUsage {
            input: 1_000,
            cache_read: 2_000,
            output: 300,
            reasoning: 0,
            total: 3_400,
        },
        models: vec!["claude-opus-5".to_string(), "claude-haiku-4-5".to_string()],
    }
}

#[tokio::test]
async fn a_session_cost_is_corrected_in_place_and_dropped_once_it_leaves_the_window() {
    let (storage, _database) = open_storage().await;
    storage
        .save_session_costs(
            CODEX,
            &[
                session("old", "2026-05-01T09:00:00Z", Some(1.0), 1.1),
                session("running", "2026-08-20T09:00:00Z", Some(2.0), 2.2),
            ],
            "2026-08-20T15:00:00Z",
        )
        .await
        .expect("store the comparisons");
    storage
        .save_session_costs(
            CODEX,
            &[session("running", "2026-08-20T09:00:00Z", Some(3.0), 3.3)],
            "2026-08-20T16:00:00Z",
        )
        .await
        .expect("store the grown session");

    storage.run_retention_at("2026-08-20T16:00:00Z").await.expect("run retention");

    let remaining: Vec<(String, f64)> =
        sqlx::query_as("SELECT session_id, reported_cost_usd FROM session_costs")
            .fetch_all(&storage.pool)
            .await
            .expect("read the surviving comparisons");
    assert_eq!(remaining, [("running".to_string(), 3.0)]);
}

#[tokio::test]
async fn a_range_answers_with_the_sessions_that_started_in_its_local_days() {
    let (storage, _database) = open_storage().await;
    storage
        .save_session_costs(
            CODEX,
            &[
                session("earlier", "2026-08-18T09:00:00Z", Some(1.0), 1.5),
                session("wanted", "2026-08-20T09:00:00Z", Some(2.0), 2.5),
                // Listed like any other, but never part of a total that compares the
                // two sides.
                session("unpriced", "2026-08-20T11:00:00Z", None, 9.0),
            ],
            "2026-08-20T15:00:00Z",
        )
        .await
        .expect("store the comparisons");
    // The rows are filtered by the local day they started on, which is the day the
    // reader picked on screen, so the expected day is read the same way.
    let day: String = sqlx::query_scalar("SELECT date(?, 'localtime')")
        .bind("2026-08-20T09:00:00Z")
        .fetch_one(&storage.pool)
        .await
        .expect("the local day of the wanted session");

    let snapshot = storage.session_costs(Some(CODEX), &day, &day).await.expect("read the range");

    assert_eq!(
        snapshot.sessions.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["unpriced", "wanted"]
    );
    let wanted = &snapshot.sessions[1];
    assert_eq!(wanted.usage.total, 3_400);
    assert_eq!(wanted.models, ["claude-opus-5", "claude-haiku-4-5"]);
    assert_eq!(wanted.duration_ms, 900_000);
    assert_eq!((snapshot.reported_cost_usd, snapshot.computed_cost_usd), (2.0, 2.5));
}

#[tokio::test]
async fn migrations_leave_only_the_tables_the_core_writes() {
    let (storage, _database) = open_storage().await;
    assert_eq!(
        storage.table_names().await.expect("read table names"),
        [
            "daily_usage",
            "devices",
            "hourly_usage",
            "limit_current",
            "limit_resets",
            "limit_rollups",
            "limit_samples",
            "provider_instances",
            "refresh_runs",
            "retention_state",
            "session_costs",
        ]
    );
}

#[tokio::test]
async fn a_sample_is_dated_by_the_reading_rather_than_the_refresh_that_carried_it() {
    let (storage, _database) = open_storage().await;
    let live = weekly_live(31.0, 1_786_800_000);
    storage.save_live(CODEX, &live, "2026-06-04T10:00:00Z").await.expect("first refresh");
    storage.save_live(CODEX, &live, "2026-06-06T10:00:00Z").await.expect("second refresh");
    let dates: Vec<String> =
        sqlx::query_scalar("SELECT observed_at FROM limit_samples ORDER BY id")
            .fetch_all(&storage.pool)
            .await
            .expect("read the samples");
    let reading = jiff::Timestamp::from_second(live.limits[0].observed_at)
        .expect("the reading's own time")
        .to_string();
    assert_eq!(
        dates,
        vec![reading.clone(), reading],
        "a reading republished by a later refresh keeps the day it was taken on"
    );
}

#[tokio::test]
async fn retention_keeps_daily_quota_summaries_without_an_hourly_layer() {
    let (storage, _database) = open_storage().await;
    let provider_id = storage.provider_id(CODEX).await.expect("read provider id");
    let observed_at = "2026-01-01T01:00:00Z";
    let expected_local_bucket: String =
        sqlx::query_scalar("SELECT strftime('%Y-%m-%dT00:00:00', ?, 'localtime')")
            .bind(observed_at)
            .fetch_one(&storage.pool)
            .await
            .expect("calculate the sample's local day");
    sqlx::query(
        "INSERT INTO limit_samples \
         (provider_instance_id, window_kind, used_percent, window_duration_mins, resets_at, observed_at) \
         VALUES (?, 'primary', 25.0, 300, 1767301200, '2026-01-01T01:00:00Z')",
    )
    .bind(provider_id)
    .execute(&storage.pool)
    .await
    .expect("insert old sample");
    sqlx::query(
        "INSERT INTO limit_rollups \
         (provider_instance_id, granularity, bucket_start, bucket_end, window_kind, window_duration_mins, \
          resets_at, reset_segment, start_used_percent, end_used_percent, min_used_percent, max_used_percent, \
          average_used_percent, sample_count) \
         VALUES (?, 'hourly', '2026-02-01T01:00:00Z', '2026-02-01T01:59:59Z', 'primary', 300, \
          1769972400, '1769972400', 20.0, 30.0, 20.0, 30.0, 25.0, 2)",
    )
    .bind(provider_id)
    .execute(&storage.pool)
    .await
    .expect("insert legacy hourly rollup");

    storage.run_retention_at("2026-08-15T00:00:00Z").await.expect("run retention");

    let samples: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM limit_samples")
        .fetch_one(&storage.pool)
        .await
        .expect("count samples");
    let hourly: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_rollups WHERE granularity = 'hourly'")
            .fetch_one(&storage.pool)
            .await
            .expect("count hourly rollups");
    let daily: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM limit_rollups WHERE granularity = 'daily'")
            .fetch_one(&storage.pool)
            .await
            .expect("count daily rollups");
    assert_eq!(samples, 0);
    assert_eq!(hourly, 0);
    assert_eq!(daily, 2, "both old samples and legacy hourly rows become daily summaries");
    let sample_bucket: String = sqlx::query_scalar(
        "SELECT bucket_start FROM limit_rollups WHERE granularity = 'daily' AND resets_at = 1767301200",
    )
    .fetch_one(&storage.pool)
    .await
    .expect("read the retained sample's bucket");
    assert_eq!(
        sample_bucket, expected_local_bucket,
        "retention keeps the sample on its local calendar day"
    );
}

#[tokio::test]
async fn a_history_refresh_replaces_only_the_days_it_parsed() {
    let (storage, _database) = open_storage().await;
    let first = HistorySnapshot {
        days: vec![day("2026-08-01", "gpt-5", 100), day("2026-08-02", "gpt-5", 200)],
        hours: Vec::new(),
    };
    storage
        .save_history(CODEX, &first, "Australia/Brisbane", "2026-08-02T00:00:00Z")
        .await
        .expect("save first history");

    // A later parse no longer sees the rotated-away session that produced 08-01.
    let second = HistorySnapshot { days: vec![day("2026-08-02", "gpt-5", 500)], hours: Vec::new() };
    storage
        .save_history(CODEX, &second, "Australia/Brisbane", "2026-08-02T01:00:00Z")
        .await
        .expect("save second history");

    let range = storage
        .load_usage_range(Some(CODEX), None, "2026-08-01", "2026-08-02")
        .await
        .expect("load range");
    assert_eq!(range.days.len(), 2, "the day outside the parse must survive");
    assert_eq!(range.days[0].usage.total, 100);
    assert_eq!(range.days[1].usage.total, 500, "the reparsed day must be replaced, not added to");
    assert_eq!(range.usage.total, 600);
}

/// The dashboard's combined view asks for no provider at all.
#[tokio::test]
async fn a_range_with_no_provider_counts_every_provider_together() {
    let (storage, _database) = open_storage().await;
    let codex = HistorySnapshot { days: vec![day("2026-08-01", "gpt-5", 100)], hours: Vec::new() };
    let claude =
        HistorySnapshot { days: vec![day("2026-08-01", "claude-opus-5", 400)], hours: Vec::new() };
    storage
        .save_history(CODEX, &codex, "Australia/Brisbane", "2026-08-01T00:00:00Z")
        .await
        .expect("save Codex history");
    storage
        .save_history(ProviderKind::Claude, &claude, "Australia/Brisbane", "2026-08-01T00:00:00Z")
        .await
        .expect("save Claude history");

    let combined = storage
        .load_usage_range(None, None, "2026-08-01", "2026-08-01")
        .await
        .expect("load combined range");
    assert_eq!(combined.usage.total, 500);
    assert_eq!(
        combined.days.len(),
        1,
        "one calendar day stays one point however many providers filled it"
    );
    assert_eq!(combined.models.len(), 2, "each provider's models keep their own row");

    let single = storage
        .load_usage_range(Some(CODEX), None, "2026-08-01", "2026-08-01")
        .await
        .expect("load Codex range");
    assert_eq!(single.usage.total, 100, "asking for one provider still answers for that one alone");
}

#[tokio::test]
async fn the_usage_start_date_follows_provider_and_device_filters() {
    let (storage, _database) = open_storage().await;
    storage
        .save_history(
            CODEX,
            &HistorySnapshot { days: vec![day("2026-08-03", "gpt-5", 100)], hours: Vec::new() },
            "Australia/Brisbane",
            "2026-08-03T10:00:00Z",
        )
        .await
        .expect("save local Codex history");
    let daily = [device_row("claude", "2026-07-01", "claude-opus-5", 400)];
    storage
        .import_device(
            &DeviceImport {
                id: "workshop",
                display_name: "Workshop",
                parser_revision: "test",
                source_modified_at: 1,
                daily: &daily,
                hourly: &[],
                resets: &[],
            },
            "2026-08-03T10:01:00Z",
        )
        .await
        .expect("import remote Claude usage");

    assert_eq!(
        storage.load_usage_start_date(None, None).await.expect("load combined start"),
        Some("2026-07-01".to_string())
    );
    assert_eq!(
        storage.load_usage_start_date(Some(CODEX), None).await.expect("load Codex start"),
        Some("2026-08-03".to_string())
    );
    assert_eq!(
        storage.load_usage_start_date(None, Some("workshop")).await.expect("load workshop start"),
        Some("2026-07-01".to_string())
    );
}

#[tokio::test]
async fn a_device_filter_narrows_daily_and_hourly_usage_without_hiding_the_split() {
    let (storage, _database) = open_storage().await;
    storage.record_local_device("This machine").await.expect("name the local device");
    storage
        .save_history(
            CODEX,
            &HistorySnapshot {
                days: vec![day("2026-08-01", "gpt-5", 100)],
                hours: vec![hour("2026-08-01T09:00", "gpt-5", 100)],
            },
            "Australia/Brisbane",
            "2026-08-01T10:00:00Z",
        )
        .await
        .expect("save local history");
    let daily = [device_row("codex", "2026-08-01", "gpt-5", 400)];
    let hourly = [device_row("codex", "2026-08-01T09:00", "gpt-5", 400)];
    storage
        .import_device(
            &DeviceImport {
                id: "workshop",
                display_name: "Workshop",
                parser_revision: "test",
                source_modified_at: 1,
                daily: &daily,
                hourly: &hourly,
                resets: &[],
            },
            "2026-08-01T10:01:00Z",
        )
        .await
        .expect("import the other device");

    let local = storage
        .load_usage_range(Some(CODEX), Some(LOCAL_DEVICE), "2026-08-01", "2026-08-01")
        .await
        .expect("load local usage");
    assert_eq!(local.usage.total, 100);
    assert_eq!(local.devices.len(), 2, "the filter choices remain visible");
    assert_eq!(local.devices.iter().map(|device| device.tokens).sum::<u64>(), 500);

    let remote = storage
        .load_usage_range(Some(CODEX), Some("workshop"), "2026-08-01", "2026-08-01")
        .await
        .expect("load remote usage");
    assert_eq!(remote.usage.total, 400);
    let remote_hours = storage
        .load_usage_hours(Some(CODEX), Some("workshop"), "2026-08-01", "2026-08-01")
        .await
        .expect("load remote hours");
    assert_eq!(remote_hours.hours[0].usage.total, 400);
}

#[tokio::test]
async fn imported_usage_makes_a_provider_available_without_local_history() {
    let (storage, _database) = open_storage().await;
    let daily = [device_row("claude", "2026-08-01", "claude-opus-5", 400)];
    storage
        .import_device(
            &DeviceImport {
                id: "workshop",
                display_name: "Workshop",
                parser_revision: "test",
                source_modified_at: 1,
                daily: &daily,
                hourly: &[],
                resets: &[],
            },
            "2026-08-01T10:01:00Z",
        )
        .await
        .expect("import remote Claude usage");

    assert_eq!(
        storage.load_usage_providers().await.expect("load providers"),
        [ProviderKind::Claude]
    );
}

#[tokio::test]
async fn a_timezone_change_rebuilds_provider_history_without_old_date_buckets() {
    let (storage, _database) = open_storage().await;
    let first = HistorySnapshot {
        days: vec![day("2026-08-01", "gpt-5", 100), day("2026-08-02", "gpt-5", 200)],
        hours: Vec::new(),
    };
    storage
        .save_history(CODEX, &first, "Australia/Brisbane", "2026-08-02T00:00:00Z")
        .await
        .expect("save Brisbane history");

    let rebucketed =
        HistorySnapshot { days: vec![day("2026-08-02", "gpt-5", 250)], hours: Vec::new() };
    storage
        .save_history(CODEX, &rebucketed, "America/New_York", "2026-08-02T01:00:00Z")
        .await
        .expect("rebuild New York history");

    let range = storage
        .load_usage_range(Some(CODEX), None, "2026-08-01", "2026-08-02")
        .await
        .expect("load range");
    assert_eq!(range.days.len(), 1, "rows bucketed in the previous timezone must not survive");
    assert_eq!(range.days[0].date, "2026-08-02");
    assert_eq!(range.days[0].usage.total, 250);
}

#[tokio::test]
async fn migration_adopts_current_timezone_without_dropping_legacy_days() {
    let (storage, _database) = open_storage().await;
    let first = HistorySnapshot {
        days: vec![day("2026-08-01", "gpt-5", 100), day("2026-08-02", "gpt-5", 200)],
        hours: Vec::new(),
    };
    storage
        .save_history(CODEX, &first, "Australia/Brisbane", "2026-08-02T00:00:00Z")
        .await
        .expect("save initial history");
    sqlx::query(
        "UPDATE provider_instances SET aggregation_timezone = NULL WHERE provider = 'codex'",
    )
    .execute(&storage.pool)
    .await
    .expect("simulate an upgraded database");

    let latest = HistorySnapshot { days: vec![day("2026-08-02", "gpt-5", 250)], hours: Vec::new() };
    storage
        .save_history(CODEX, &latest, "Australia/Brisbane", "2026-08-02T01:00:00Z")
        .await
        .expect("adopt the current timezone");

    let range = storage
        .load_usage_range(Some(CODEX), None, "2026-08-01", "2026-08-02")
        .await
        .expect("load range");
    assert_eq!(
        range.days.len(),
        2,
        "an unknown legacy timezone must not trigger destructive cleanup"
    );
    assert_eq!(range.days[0].usage.total, 100);
    assert_eq!(range.days[1].usage.total, 250);
}

#[tokio::test]
async fn a_usage_range_reports_only_the_requested_days() {
    let (storage, _database) = open_storage().await;
    let history = HistorySnapshot {
        days: vec![day("2026-08-01", "gpt-5", 100), day("2026-08-02", "gpt-5-codex", 300)],
        hours: Vec::new(),
    };
    storage
        .save_history(CODEX, &history, "Australia/Brisbane", "2026-08-02T00:00:00Z")
        .await
        .expect("save history");

    let range = storage
        .load_usage_range(Some(CODEX), None, "2026-08-02", "2026-08-02")
        .await
        .expect("load range");
    assert_eq!(range.days.len(), 1);
    assert_eq!(range.usage.total, 300);
    assert_eq!(range.models.len(), 1);
    assert_eq!(range.models[0].model, "gpt-5-codex");
    assert_eq!(range.api_equivalent_cost_usd, Some(1.5));
}

#[tokio::test]
async fn a_restored_snapshot_keeps_the_window_naming_of_the_live_read() {
    let (storage, _database) = open_storage().await;
    let live = LiveSnapshot {
        plan_type: Some("plus".to_string()),
        earned_reset_count: Some(2),
        earned_reset_expires_at: None,
        limits: vec![LimitWindow {
            kind: LimitKind::Primary,
            label: LimitKind::Primary.window_label(Some(300)),
            used_percent: Some(40.0),
            window_duration_mins: Some(300),
            resets_at: Some(1_800_000_000),
            source: WindowSource::AppServer,
            observed_at: jiff::Timestamp::now().as_second(),
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
            pace: PaceLevel::OnTrack,
        }],
    };
    storage.save_live(CODEX, &live, "2026-08-11T00:00:00Z").await.expect("save live");

    let snapshot = storage.load_snapshot(CODEX).await.expect("load snapshot");
    assert_eq!(snapshot.plan_type.as_deref(), Some("plus"));
    assert_eq!(snapshot.limits.len(), 1);
    assert_eq!(snapshot.limits[0].label, "5-hour window");
    assert_eq!(snapshot.limits[0].used_percent, Some(40.0));
    assert_eq!(snapshot.freshness, Freshness::Fresh, "freshness follows the stored observation");
}

const WEEK_MINUTES: i64 = 10_080;

fn weekly_live(used_percent: f64, resets_at: i64) -> LiveSnapshot {
    LiveSnapshot {
        plan_type: Some("plus".to_string()),
        earned_reset_count: Some(0),
        earned_reset_expires_at: None,
        limits: vec![LimitWindow {
            kind: LimitKind::Primary,
            label: LimitKind::Primary.window_label(Some(WEEK_MINUTES)),
            used_percent: Some(used_percent),
            window_duration_mins: Some(WEEK_MINUTES),
            resets_at: Some(resets_at),
            source: WindowSource::AppServer,
            observed_at: resets_at - WEEK_MINUTES * 60,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
            pace: PaceLevel::OnTrack,
        }],
    }
}

/// The same reading, taken at a stated moment rather than at the one `weekly_live`
/// derives from the window's expiry.
fn weekly_live_read_at(used_percent: f64, resets_at: i64, observed_at: &str) -> LiveSnapshot {
    let mut live = weekly_live(used_percent, resets_at);
    live.limits[0].observed_at =
        observed_at.parse::<jiff::Timestamp>().expect("a reading time").as_second();
    live
}

#[tokio::test]
async fn a_window_that_restarts_early_is_recorded_against_the_reading_it_replaced() {
    let (storage, _database) = open_storage().await;
    // Two days of the weekly window were spent and four remained.
    storage
        .save_live(CODEX, &weekly_live(52.0, 1_786_800_000), "2026-08-10T15:00:00Z")
        .await
        .expect("save the reading before the restart");
    storage
        .save_live(CODEX, &weekly_live(0.0, 1_787_026_583), "2026-08-11T09:29:00Z")
        .await
        .expect("save the reading after the restart");

    let snapshot = storage.load_snapshot(CODEX).await.expect("load snapshot");
    assert_eq!(snapshot.recent_resets.len(), 1);
    let event = &snapshot.recent_resets[0];
    assert_eq!(event.classification, ResetClassification::Unplanned);
    assert_eq!(event.used_percent_before, 52.0);
    assert_eq!(event.anchored_at, 1_787_026_583 - WEEK_MINUTES * 60);
    assert_eq!(event.previous_resets_at, 1_786_800_000);
}

#[tokio::test]
async fn a_window_ageing_out_earlier_requests_is_not_recorded_as_a_restart() {
    let (storage, _database) = open_storage().await;
    let expiry = 1_786_800_000;
    storage
        .save_live(CODEX, &weekly_live(15.0, expiry), "2026-06-04T10:00:00Z")
        .await
        .expect("save first");
    storage
        .save_live(CODEX, &weekly_live(4.0, expiry + 7_200), "2026-06-04T15:00:00Z")
        .await
        .expect("save second");
    assert!(storage.load_recent_resets(CODEX).await.expect("load resets").is_empty());
}

/// The dates the samples fall on are the machine's own, so the assertions here are
/// about the shape of a day rather than which day it is: the range is wide enough that
/// every reading lands inside it whatever the time zone.
#[tokio::test]
async fn a_day_of_quota_readings_is_summarised_by_its_peak() {
    let (storage, _database) = open_storage().await;
    let expiry = 1_786_800_000;
    for (percent, observed_at) in [
        (12.0, "2026-06-04T02:00:00Z"),
        (61.0, "2026-06-04T11:00:00Z"),
        (37.0, "2026-06-04T20:00:00Z"),
    ] {
        storage
            .save_live(CODEX, &weekly_live_read_at(percent, expiry, observed_at), observed_at)
            .await
            .expect("save a reading");
    }

    let history = storage
        .load_quota_history(CODEX, "2026-06-01", "2026-06-08")
        .await
        .expect("load quota history");
    assert_eq!(history.windows.len(), 1, "one window was read all day");
    let window = &history.windows[0];
    assert_eq!(window.kind, LimitKind::Primary);
    let highest = window.points.iter().map(|point| point.peak_used_percent).fold(0.0, f64::max);
    assert_eq!(highest, 61.0, "a day is summarised by the fullest the window got");
    assert!(window.points.len() <= 2, "the readings span at most two local days");
    for point in &window.points {
        assert!(
            [12.0, 61.0, 37.0].contains(&point.peak_used_percent),
            "a point is one of the readings, never an average of them",
        );
    }
    assert!(history.resets.is_empty(), "nothing restarted");
}

#[tokio::test]
async fn a_range_with_no_readings_has_no_windows_to_draw() {
    let (storage, _database) = open_storage().await;
    storage
        .save_live(CODEX, &weekly_live(20.0, 1_786_800_000), "2026-06-04T10:00:00Z")
        .await
        .expect("save a reading");
    let history = storage
        .load_quota_history(CODEX, "2026-07-01", "2026-07-07")
        .await
        .expect("load quota history");
    assert!(history.windows.is_empty());
}

/// Claude's windows come from two sources at once. Only the status line publishes the
/// window; the session logs recover its timing from request times, and a reading of that
/// kind describes a different thing well enough to look like a restart beside a
/// published one.
#[tokio::test]
async fn a_restart_is_inferred_only_from_the_source_that_publishes_the_window() {
    let (storage, _database) = open_storage().await;
    let claude = ProviderKind::Claude;
    let logged = |used, resets_at| {
        let mut live = weekly_live(used, resets_at);
        live.limits[0].source = WindowSource::SessionLog;
        live
    };
    storage
        .save_live(claude, &logged(52.0, 1_786_800_000), "2026-08-10T15:00:00Z")
        .await
        .expect("save the earlier log-derived reading");
    storage
        .save_live(claude, &logged(0.0, 1_787_026_583), "2026-08-11T09:29:00Z")
        .await
        .expect("save the later log-derived reading");
    assert!(
        storage.load_recent_resets(claude).await.expect("load Claude resets").is_empty(),
        "a window recovered from local logs cannot evidence a restart"
    );
}

/// Claude Code publishes the five-hour window with a restart but no percentage while one
/// closes, and the session-log fallback fills the gap with a window it can time but not
/// measure. Storing that reading would replace the last published percentage, leaving
/// the first reading of the new window nothing to be a restart against.
#[tokio::test]
async fn a_reading_without_an_allowance_leaves_the_measured_window_it_cannot_replace() {
    let (storage, _database) = open_storage().await;
    let claude = ProviderKind::Claude;
    let published = |used, resets_at| {
        let mut live = weekly_live(used, resets_at);
        live.limits[0].source = WindowSource::StatusLine;
        live
    };
    let unmeasured = |resets_at| {
        let mut live = published(0.0, resets_at);
        live.limits[0].used_percent = None;
        live.limits[0].source = WindowSource::SessionLog;
        live
    };
    storage
        .save_live(claude, &published(78.0, 1_786_800_000), "2026-08-10T15:00:00Z")
        .await
        .expect("save the published reading");
    storage
        .save_live(claude, &unmeasured(1_786_810_000), "2026-08-11T09:28:00Z")
        .await
        .expect("save the reading that carries no percentage");

    let snapshot = storage.load_snapshot(claude).await.expect("load snapshot");
    assert_eq!(snapshot.limits.len(), 1);
    assert_eq!(
        snapshot.limits[0].used_percent,
        Some(78.0),
        "the measured reading stands until another measurement replaces it",
    );
    assert_eq!(snapshot.limits[0].resets_at, Some(1_786_800_000));

    storage
        .save_live(claude, &published(1.0, 1_787_026_583), "2026-08-11T09:29:00Z")
        .await
        .expect("save the reading after the restart");
    let events = storage.load_recent_resets(claude).await.expect("load Claude resets");
    assert_eq!(events.len(), 1, "the restart is still recognised across the gap");
    assert_eq!(events[0].used_percent_before, 78.0);
}

/// A window nothing has ever measured is still worth storing: the session logs are the
/// only source when Claude Code has never run beside QuotaStation.
#[tokio::test]
async fn a_window_no_source_has_measured_is_stored_from_the_timing_alone() {
    let (storage, _database) = open_storage().await;
    let claude = ProviderKind::Claude;
    let mut live = weekly_live(0.0, 1_786_800_000);
    live.limits[0].used_percent = None;
    live.limits[0].source = WindowSource::SessionLog;
    storage.save_live(claude, &live, "2026-08-10T15:00:00Z").await.expect("save the reading");

    let snapshot = storage.load_snapshot(claude).await.expect("load snapshot");
    assert_eq!(snapshot.limits.len(), 1, "the window is known even without an allowance");
    assert_eq!(snapshot.limits[0].used_percent, None);
}

/// The quota Claude Code hands its status line is server-published, exactly like Codex's
/// app-server answer, so a restart of one of those windows is recorded the same way.
#[tokio::test]
async fn a_claude_window_the_status_line_published_records_its_restart() {
    let (storage, _database) = open_storage().await;
    let claude = ProviderKind::Claude;
    let published = |used, resets_at| {
        let mut live = weekly_live(used, resets_at);
        live.limits[0].source = WindowSource::StatusLine;
        live
    };
    storage
        .save_live(claude, &published(78.0, 1_786_800_000), "2026-08-10T15:00:00Z")
        .await
        .expect("save the earlier published reading");
    storage
        .save_live(claude, &published(1.0, 1_787_026_583), "2026-08-11T09:29:00Z")
        .await
        .expect("save the reading after the restart");

    let events = storage.load_recent_resets(claude).await.expect("load Claude resets");
    assert_eq!(events.len(), 1, "the restart is recorded");
    assert_eq!(events[0].used_percent_before, 78.0);
}

/// A local hour yesterday, as both the key its usage is stored under and the instant a
/// restart anchored there falls at. The day moves with the clock because a window's total
/// is only rebuilt while the hours behind it are still kept.
fn yesterday_at(hour: i8) -> (String, i64) {
    let date = jiff::Zoned::now().date().yesterday().expect("a previous day");
    let epoch = date
        .at(hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::system())
        .expect("a resolvable local hour")
        .timestamp()
        .as_second();
    (format!("{date}T{hour:02}:00"), epoch)
}

/// Two restarts of a five-hour window and the hours of usage around them. The expiry is
/// the one the closed window was still publishing when the second restart was read, which
/// is what says whether that window ran to its end or expired unused some time before it.
async fn two_restarts(
    storage: &Storage,
    first: i64,
    second: i64,
    published_expiry: i64,
    hours: Vec<HistoryHour>,
) {
    let window = |observed_at, used_percent, resets_at| WindowObservation {
        observed_at,
        kind: LimitKind::Primary,
        used_percent,
        window_duration_mins: 300,
        resets_at,
    };
    storage
        .backfill_resets(
            CODEX,
            &[
                window(first - 600, 40.0, first + 1_200),
                window(first + 600, 0.0, first + 18_000),
                window(second - 600, 55.0, published_expiry),
                window(second + 600, 2.0, second + 18_000),
            ],
            "2026-08-20T12:00:00Z",
        )
        .await
        .expect("record the two restarts");
    let date = jiff::Zoned::now().date().yesterday().expect("a previous day").to_string();
    storage
        .save_history(
            CODEX,
            &HistorySnapshot { days: vec![day(&date, "gpt-5-codex", 2_200)], hours },
            "Australia/Sydney",
            "2026-08-20T12:00:00Z",
        )
        .await
        .expect("save history");
}

/// The reset list says how much was spent inside the window each restart closed, and the
/// hours are credited to the window that was running when they opened: the hour before
/// the window began belongs to the window before it, and the hour the restart fell in
/// belongs to the window that started there.
#[tokio::test]
async fn a_restart_reports_the_tokens_the_window_it_closed_carried() {
    let (storage, _database) = open_storage().await;
    let (before, _) = yesterday_at(5);
    let (opening, first) = yesterday_at(6);
    let (middle, _) = yesterday_at(9);
    let (closing, second) = yesterday_at(11);
    two_restarts(
        &storage,
        first,
        second,
        second,
        vec![
            hour(&before, "gpt-5-codex", 1_000),
            hour(&opening, "gpt-5-codex", 200),
            hour(&middle, "gpt-5-codex", 300),
            hour(&closing, "gpt-5-codex", 700),
        ],
    )
    .await;

    let events = storage.load_recent_resets(CODEX).await.expect("load resets");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].anchored_at, second, "the newest restart is listed first");
    assert_eq!(events[0].tokens_in_window, Some(500));
}

/// Codex moves a window between the primary and secondary slot, so the restart before
/// this one is the one of the same length rather than the one in the same slot. Pairing
/// by slot hands the weekly window the five-hour restart that happened in between and
/// reports a couple of hours of work as everything the week carried.
#[tokio::test]
async fn a_weekly_window_is_paired_with_the_weekly_restart_before_it() {
    let (storage, _database) = open_storage().await;
    let (_, week_first) = yesterday_at(4);
    let (early, _) = yesterday_at(5);
    let (_, five_hour) = yesterday_at(9);
    let (late, _) = yesterday_at(10);
    let (_, week_second) = yesterday_at(11);
    let window = |observed_at, kind, used_percent, window_duration_mins, resets_at| {
        WindowObservation { observed_at, kind, used_percent, window_duration_mins, resets_at }
    };
    storage
        .backfill_resets(
            CODEX,
            &[
                // The weekly window starts in the secondary slot and comes back in the
                // primary one, with a five-hour restart of its own in between.
                window(week_first - 600, LimitKind::Secondary, 60.0, 10_080, week_first + 1_200),
                window(week_first + 600, LimitKind::Secondary, 1.0, 10_080, week_first + 604_800),
                window(five_hour - 600, LimitKind::Primary, 80.0, 300, five_hour + 1_200),
                window(five_hour + 600, LimitKind::Primary, 2.0, 300, five_hour + 18_000),
                window(week_second - 600, LimitKind::Primary, 70.0, 10_080, week_second + 1_200),
                window(week_second + 600, LimitKind::Primary, 3.0, 10_080, week_second + 604_800),
            ],
            "2026-08-20T12:00:00Z",
        )
        .await
        .expect("record the restarts");
    let date = jiff::Zoned::now().date().yesterday().expect("a previous day").to_string();
    storage
        .save_history(
            CODEX,
            &HistorySnapshot {
                days: vec![day(&date, "gpt-5-codex", 500)],
                hours: vec![hour(&early, "gpt-5-codex", 200), hour(&late, "gpt-5-codex", 300)],
            },
            "Australia/Sydney",
            "2026-08-20T12:00:00Z",
        )
        .await
        .expect("save history");

    let events = storage.load_recent_resets(CODEX).await.expect("load resets");
    let weekly = events
        .iter()
        .find(|event| event.anchored_at == week_second)
        .expect("the second weekly restart");
    assert_eq!(
        weekly.tokens_in_window,
        Some(500),
        "the week runs from the weekly restart, not from the five-hour one inside it",
    );
}

/// A restart that went unrecorded leaves a gap far longer than the window it closed, and
/// crediting the whole gap to it would report work three windows ago as this one's.
#[tokio::test]
async fn a_window_is_credited_with_no_more_than_its_own_length() {
    let (storage, _database) = open_storage().await;
    let (_, midnight) = yesterday_at(0);
    let (long_before, _) = yesterday_at(3);
    let (opening, _) = yesterday_at(6);
    let (middle, _) = yesterday_at(9);
    let (_, second) = yesterday_at(11);
    two_restarts(
        &storage,
        midnight,
        second,
        second,
        vec![
            hour(&long_before, "gpt-5-codex", 1_000),
            hour(&opening, "gpt-5-codex", 200),
            hour(&middle, "gpt-5-codex", 300),
        ],
    )
    .await;

    let events = storage.load_recent_resets(CODEX).await.expect("load resets");
    assert_eq!(events[0].anchored_at, second);
    assert_eq!(
        events[0].tokens_in_window,
        Some(500),
        "the eleven-hour gap is read as the five-hour window it can have been",
    );
}

/// A window that expired unused is followed by however long it took for the next request
/// to anchor the next one. That idle gap belongs to neither window, and reading the
/// closed one as far as the restart would have credited it with the whole of it.
#[tokio::test]
async fn a_window_that_expired_before_the_next_one_began_stops_at_its_expiry() {
    let (storage, _database) = open_storage().await;
    let (opening, first) = yesterday_at(2);
    let (middle, _) = yesterday_at(6);
    let (after_expiry, _) = yesterday_at(10);
    let (_, second) = yesterday_at(13);
    two_restarts(
        &storage,
        first,
        second,
        first + 18_000,
        vec![
            hour(&opening, "gpt-5-codex", 400),
            hour(&middle, "gpt-5-codex", 600),
            hour(&after_expiry, "gpt-5-codex", 999),
        ],
    )
    .await;

    let events = storage.load_recent_resets(CODEX).await.expect("load resets");
    assert_eq!(events[0].anchored_at, second);
    assert_eq!(
        events[0].tokens_in_window,
        Some(1_000),
        "an hour after the window stopped counting is not part of what it carried",
    );
}

#[tokio::test]
async fn the_backfill_recovers_restarts_from_readings_taken_while_the_app_was_closed() {
    let (storage, _database) = open_storage().await;
    let anchor = 1_786_500_000;
    let observations = vec![
        WindowObservation {
            observed_at: anchor,
            kind: LimitKind::Primary,
            used_percent: 25.0,
            window_duration_mins: WEEK_MINUTES,
            resets_at: anchor + 3 * 86_400,
        },
        WindowObservation {
            observed_at: anchor + 3_600,
            kind: LimitKind::Primary,
            used_percent: 0.0,
            window_duration_mins: WEEK_MINUTES,
            resets_at: anchor + 3_600 + WEEK_MINUTES * 60,
        },
    ];
    let recorded = storage
        .backfill_resets(CODEX, &observations, "2026-08-12T00:00:00Z")
        .await
        .expect("run the backfill");
    assert_eq!(recorded, 1);

    // A second scan sees the same readings again and must not duplicate the event.
    storage
        .backfill_resets(CODEX, &observations, "2026-08-12T01:00:00Z")
        .await
        .expect("rerun the backfill");
    let events = storage.load_recent_resets(CODEX).await.expect("load resets");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].classification, ResetClassification::Unplanned);
    assert!(storage.reset_backfill_start(CODEX).await.expect("read cursor").is_some());
}
