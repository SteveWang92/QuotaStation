//! How long the normalized data is kept: the hourly layer is rolled up into the daily one
//! and the readings behind it are dropped once they leave the window.

use anyhow::Result;
use sqlx::Row;

use crate::domain::{HOURLY_HISTORY_DAYS, RetentionDiagnostics};
use crate::sanitize::sanitize_error;

use super::{SAMPLE_HISTORY_DAYS, SESSION_COST_HISTORY_DAYS, Storage};

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
        sqlx::query(
            "INSERT INTO limit_rollups (provider_instance_id, granularity, bucket_start, bucket_end, window_kind, \
               window_duration_mins, resets_at, reset_segment, start_used_percent, end_used_percent, min_used_percent, \
               max_used_percent, average_used_percent, sample_count) \
             SELECT h.provider_instance_id, 'daily', strftime('%Y-%m-%dT00:00:00', h.bucket_start, 'localtime'), \
               strftime('%Y-%m-%dT23:59:59.999999999', h.bucket_start, 'localtime'), h.window_kind, h.window_duration_mins, \
               h.resets_at, h.reset_segment, \
               (SELECT x.start_used_percent FROM limit_rollups x WHERE x.provider_instance_id = h.provider_instance_id \
                 AND x.granularity = 'hourly' AND date(x.bucket_start, 'localtime') = date(h.bucket_start, 'localtime') \
                 AND x.window_kind = h.window_kind AND x.reset_segment = h.reset_segment ORDER BY x.bucket_start LIMIT 1), \
               (SELECT x.end_used_percent FROM limit_rollups x WHERE x.provider_instance_id = h.provider_instance_id \
                 AND x.granularity = 'hourly' AND date(x.bucket_start, 'localtime') = date(h.bucket_start, 'localtime') \
                 AND x.window_kind = h.window_kind AND x.reset_segment = h.reset_segment ORDER BY x.bucket_start DESC LIMIT 1), \
               MIN(h.min_used_percent), MAX(h.max_used_percent), \
               SUM(h.average_used_percent * h.sample_count) / NULLIF(SUM(CASE WHEN h.average_used_percent IS NOT NULL THEN h.sample_count ELSE 0 END), 0), \
               SUM(h.sample_count) \
             FROM limit_rollups h WHERE h.granularity = 'hourly' \
             GROUP BY h.provider_instance_id, date(h.bucket_start, 'localtime'), h.window_kind, h.window_duration_mins, h.resets_at, h.reset_segment \
             ON CONFLICT(provider_instance_id, granularity, bucket_start, window_kind, reset_segment) DO UPDATE SET \
               bucket_end=excluded.bucket_end, window_duration_mins=excluded.window_duration_mins, resets_at=excluded.resets_at, \
               start_used_percent=excluded.start_used_percent, end_used_percent=excluded.end_used_percent, \
               min_used_percent=excluded.min_used_percent, max_used_percent=excluded.max_used_percent, \
               average_used_percent=excluded.average_used_percent, sample_count=excluded.sample_count",
        ).execute(&mut *tx).await?;

        // Keep recent readings at source granularity, then preserve only a daily summary.
        self.roll_up_samples(
            &mut tx,
            "daily",
            "%Y-%m-%dT00:00:00",
            "%Y-%m-%dT23:59:59.999999999",
            &format!("-{SAMPLE_HISTORY_DAYS} days"),
            now,
        )
        .await?;

        sqlx::query(&format!(
            "DELETE FROM limit_samples WHERE datetime(observed_at) < datetime(?, '-{SAMPLE_HISTORY_DAYS} days')"
        ))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM limit_rollups WHERE granularity = 'hourly'")
            .execute(&mut *tx)
            .await?;
        // Hourly usage only exists to draw a range of a few days; past that window the
        // daily rows are the whole record, so the hourly ones are dropped rather than
        // rolled up into a summary that already exists.
        sqlx::query(&format!(
            "DELETE FROM hourly_usage WHERE date(hour_start) < date(?, 'localtime', '-{HOURLY_HISTORY_DAYS} days')"
        ))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "DELETE FROM session_costs \
             WHERE datetime(session_started_at) < datetime(?, '-{SESSION_COST_HISTORY_DAYS} days')"
        ))
        .bind(now)
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

    async fn roll_up_samples(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        granularity: &str,
        bucket_start: &str,
        bucket_end: &str,
        cutoff: &str,
        now: &str,
    ) -> Result<()> {
        let query = format!(
            "INSERT INTO limit_rollups (provider_instance_id, granularity, bucket_start, bucket_end, window_kind, \
               window_duration_mins, resets_at, reset_segment, start_used_percent, end_used_percent, min_used_percent, \
               max_used_percent, average_used_percent, sample_count) \
             SELECT s.provider_instance_id, ?, strftime('{bucket_start}', s.observed_at, 'localtime'), strftime('{bucket_end}', s.observed_at, 'localtime'), \
               s.window_kind, s.window_duration_mins, s.resets_at, COALESCE(CAST(s.resets_at AS TEXT), 'none'), \
               (SELECT x.used_percent FROM limit_samples x WHERE x.provider_instance_id=s.provider_instance_id \
                 AND strftime('{bucket_start}', x.observed_at, 'localtime')=strftime('{bucket_start}', s.observed_at, 'localtime') AND x.window_kind=s.window_kind \
                 AND x.window_duration_mins IS s.window_duration_mins AND x.resets_at IS s.resets_at ORDER BY x.observed_at, x.id LIMIT 1), \
               (SELECT x.used_percent FROM limit_samples x WHERE x.provider_instance_id=s.provider_instance_id \
                 AND strftime('{bucket_start}', x.observed_at, 'localtime')=strftime('{bucket_start}', s.observed_at, 'localtime') AND x.window_kind=s.window_kind \
                 AND x.window_duration_mins IS s.window_duration_mins AND x.resets_at IS s.resets_at ORDER BY x.observed_at DESC, x.id DESC LIMIT 1), \
               MIN(s.used_percent), MAX(s.used_percent), AVG(s.used_percent), COUNT(*) FROM limit_samples s \
             WHERE datetime(s.observed_at) < datetime(?, '{cutoff}') \
             GROUP BY s.provider_instance_id, strftime('{bucket_start}', s.observed_at, 'localtime'), s.window_kind, s.window_duration_mins, s.resets_at \
             ON CONFLICT(provider_instance_id, granularity, bucket_start, window_kind, reset_segment) DO UPDATE SET \
               bucket_end=excluded.bucket_end, window_duration_mins=excluded.window_duration_mins, resets_at=excluded.resets_at, \
               start_used_percent=excluded.start_used_percent, end_used_percent=excluded.end_used_percent, min_used_percent=excluded.min_used_percent, \
               max_used_percent=excluded.max_used_percent, average_used_percent=excluded.average_used_percent, sample_count=excluded.sample_count"
        );
        sqlx::query(&query).bind(granularity).bind(now).execute(&mut **tx).await?;
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
