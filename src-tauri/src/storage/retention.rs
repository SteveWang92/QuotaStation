//! How long the normalized data is kept. Quota readings, daily usage, restarts and session
//! costs are kept for good: together they grow by tens of megabytes a year, and each is a
//! record nothing can rebuild once the logs behind it are gone. Only what exists for a short
//! view or for diagnostics is dropped — hourly usage past the window the parser fills, and
//! old refresh records.

use std::collections::BTreeMap;

use anyhow::Result;
use sqlx::Row;

use crate::domain::{HOURLY_HISTORY_DAYS, RetentionDiagnostics};
use crate::sanitize::sanitize_error;

use super::Storage;

impl Storage {
    pub async fn run_retention_if_due(&self) -> Result<()> {
        let now = jiff::Timestamp::now();
        let last_completed: Option<String> = sqlx::query_scalar(
            "SELECT last_completed_at FROM retention_state WHERE job_name = 'normalized_data'",
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        if last_completed
            .as_deref()
            .and_then(|value| value.parse::<jiff::Timestamp>().ok())
            .is_some_and(|last| now.duration_since(last) < jiff::SignedDuration::from_hours(24))
        {
            return Ok(());
        }

        if let Err(error) = self.run_retention_at(&now.to_string()).await {
            let message = sanitize_error(&error.to_string(), "Retention failed");
            sqlx::query(
                "INSERT INTO retention_state (job_name, last_status, last_error) VALUES ('normalized_data', 'failed', ?) \
                 ON CONFLICT(job_name) DO UPDATE SET last_status = excluded.last_status, last_error = excluded.last_error",
            )
            .bind(&message)
            .execute(&self.pool)
            .await?;
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn run_retention_at(&self, now: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Collapse any hourly rows created by older versions into the long-lived daily
        // summary before removing that superseded intermediate layer. Reset segments remain
        // separate so a day's boundary values do not cross a detected restart.
        Self::roll_up_legacy_hours(&mut tx).await?;
        sqlx::query("DELETE FROM limit_rollups WHERE granularity = 'hourly'")
            .execute(&mut *tx)
            .await?;
        // Hourly usage only exists to draw a range of a few days; past that window the
        // daily rows are the whole record, so the hourly ones are dropped rather than
        // rolled up into a summary that already exists. The window is counted in the
        // application zone's days, which are what the hour keys are.
        let today = now.parse::<jiff::Timestamp>()?.to_zoned(crate::clock::zone()).date();
        sqlx::query("DELETE FROM hourly_usage WHERE date(hour_start) < ?")
            .bind(today.saturating_sub(jiff::Span::new().days(HOURLY_HISTORY_DAYS)).to_string())
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "DELETE FROM refresh_runs WHERE id NOT IN (SELECT MAX(id) FROM refresh_runs GROUP BY provider_instance_id, acquisition_path) \
             AND ((status = 'succeeded' AND datetime(completed_at) < datetime(?, '-30 days')) \
               OR (status = 'failed' AND datetime(completed_at) < datetime(?, '-180 days')))",
        ).bind(now).bind(now).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO retention_state (job_name, last_completed_at, last_status, last_error) \
             VALUES ('normalized_data', ?, 'succeeded', NULL) ON CONFLICT(job_name) DO UPDATE SET \
             last_completed_at=excluded.last_completed_at, last_status=excluded.last_status, last_error=NULL",
        ).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn roll_up_legacy_hours(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<()> {
        let rows = sqlx::query(
            "SELECT provider_instance_id, unixepoch(bucket_start) AS started, window_kind, \
             window_duration_mins, resets_at, reset_segment, start_used_percent, end_used_percent, \
             min_used_percent, max_used_percent, average_used_percent, sample_count \
             FROM limit_rollups WHERE granularity = 'hourly' ORDER BY bucket_start",
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut days: BTreeMap<RollupKey, DailyRollup> = BTreeMap::new();
        for row in rows {
            let Some(day) = crate::clock::day_key(row.try_get("started")?) else { continue };
            let summary = days
                .entry(RollupKey {
                    provider_id: row.try_get("provider_instance_id")?,
                    day,
                    kind: row.try_get("window_kind")?,
                    duration: row.try_get("window_duration_mins")?,
                    resets_at: row.try_get("resets_at")?,
                    segment: row.try_get("reset_segment")?,
                })
                .or_default();
            let count: i64 = row.try_get("sample_count")?;
            summary.first.get_or_insert(row.try_get("start_used_percent")?);
            summary.last = row.try_get("end_used_percent")?;
            summary.count += count;
            summary.measure(
                row.try_get("min_used_percent")?,
                row.try_get("max_used_percent")?,
                row.try_get("average_used_percent")?,
                count,
            );
        }
        for (key, summary) in days {
            summary.write(tx, &key).await?;
        }
        Ok(())
    }
    pub async fn load_retention_diagnostics(&self) -> Result<RetentionDiagnostics> {
        let row = sqlx::query("SELECT last_completed_at, last_status, last_error FROM retention_state WHERE job_name = 'normalized_data'")
            .fetch_optional(&self.pool).await?;
        Ok(match row {
            Some(row) => RetentionDiagnostics {
                status: row.get("last_status"),
                last_completed_at: row.try_get("last_completed_at")?,
                error: row.try_get("last_error")?,
            },
            None => RetentionDiagnostics {
                status: "pending".into(),
                last_completed_at: None,
                error: None,
            },
        })
    }
}

/// What one daily quota summary is keyed by: a provider's window on one local day, split
/// wherever its published expiry moved.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct RollupKey {
    provider_id: i64,
    day: String,
    kind: String,
    duration: Option<i64>,
    resets_at: Option<i64>,
    segment: String,
}

/// One day of a window's readings, as they are folded in oldest first.
#[derive(Default)]
struct DailyRollup {
    first: Option<Option<f64>>,
    last: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    weighted_sum: f64,
    measured: i64,
    count: i64,
}

impl DailyRollup {
    /// Folds in a minimum, maximum and average covering `samples` readings. An average that
    /// is unknown carries no weight, as `AVG` ignores a null.
    fn measure(&mut self, min: Option<f64>, max: Option<f64>, average: Option<f64>, samples: i64) {
        if let Some(min) = min {
            self.min = Some(self.min.map_or(min, |current| current.min(min)));
        }
        if let Some(max) = max {
            self.max = Some(self.max.map_or(max, |current| current.max(max)));
        }
        if let Some(average) = average {
            self.weighted_sum += average * samples as f64;
            self.measured += samples;
        }
    }

    async fn write(
        self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        key: &RollupKey,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO limit_rollups (provider_instance_id, granularity, bucket_start, bucket_end, window_kind, \
               window_duration_mins, resets_at, reset_segment, start_used_percent, end_used_percent, min_used_percent, \
               max_used_percent, average_used_percent, sample_count) \
             VALUES (?, 'daily', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(provider_instance_id, granularity, bucket_start, window_kind, reset_segment) DO UPDATE SET \
               bucket_end=excluded.bucket_end, window_duration_mins=excluded.window_duration_mins, resets_at=excluded.resets_at, \
               start_used_percent=excluded.start_used_percent, end_used_percent=excluded.end_used_percent, min_used_percent=excluded.min_used_percent, \
               max_used_percent=excluded.max_used_percent, average_used_percent=excluded.average_used_percent, sample_count=excluded.sample_count",
        )
        .bind(key.provider_id)
        .bind(format!("{}T00:00:00", key.day))
        .bind(format!("{}T23:59:59.999999999", key.day))
        .bind(&key.kind)
        .bind(key.duration)
        .bind(key.resets_at)
        .bind(&key.segment)
        .bind(self.first.flatten())
        .bind(self.last)
        .bind(self.min)
        .bind(self.max)
        .bind((self.measured > 0).then(|| self.weighted_sum / self.measured as f64))
        .bind(self.count)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}
