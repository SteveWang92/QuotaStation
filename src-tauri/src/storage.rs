use std::{collections::BTreeMap, path::Path, str::FromStr};

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

use crate::domain::{
    AcquisitionDiagnostics, CCUSAGE_REVISION, DailyModelUsage, DailyUsagePoint, Freshness,
    HistorySnapshot, LimitKind, LimitResetEvent, LimitWindow, LiveSnapshot, ModelUsage,
    PRICING_CATALOG_REVISION, ProviderSnapshot, ResetClassification, RetentionDiagnostics,
    TokenUsage, UsageRangeSnapshot,
};
use crate::providers::ProviderKind;
use crate::resets::{ResetTracker, WindowObservation, detect};
use crate::sanitize::sanitize_error;

/// The Codex daily report aggregates every service tier into one row per model, so the
/// tier dimension of `daily_usage` records that the row spans tiers rather than guessing
/// one. A per-tier writer must use the tier it observed, never this value.
const AGGREGATE_SERVICE_TIER: &str = "mixed";

/// The rollout scan skips files older than its previous run, with this much overlap so a
/// window that reset across the boundary still has an earlier reading to compare against.
const BACKFILL_OVERLAP_HOURS: i64 = 48;

/// How many restarts the surfaces are given. They annotate the window running now and
/// list the ones before it, neither of which needs the whole history.
const RECENT_RESET_LIMIT: i64 = 8;

fn kind_column(kind: LimitKind) -> &'static str {
    match kind {
        LimitKind::Primary => "primary",
        LimitKind::Secondary => "secondary",
    }
}

fn parse_kind(value: &str) -> Option<LimitKind> {
    match value {
        "primary" => Some(LimitKind::Primary),
        "secondary" => Some(LimitKind::Secondary),
        _ => None,
    }
}

/// Each provider replays its own logs, so the scan watermarks cannot share a row.
fn backfill_job_name(provider: ProviderKind) -> String {
    format!("{}_reset_backfill", provider.key())
}

fn epoch_seconds(value: &str) -> Option<i64> {
    value.parse::<jiff::Timestamp>().ok().map(|timestamp| timestamp.as_second())
}

#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
}

impl Storage {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create application data directory")?;
        }
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    async fn provider_id(&self, provider: ProviderKind) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT id FROM provider_instances WHERE provider = ?")
                .bind(provider.key())
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn save_live(
        &self,
        provider: ProviderKind,
        live: &LiveSnapshot,
        observed_at: &str,
    ) -> Result<()> {
        let provider_id = self.provider_id(provider).await?;
        // The row about to be overwritten is the only record of what the window looked
        // like before this reading, so a restart has to be recognised here.
        let previous = self.load_current_observations(provider_id).await?;
        let mut tx = self.pool.begin().await?;
        if let Some(now) = epoch_seconds(observed_at) {
            for limit in &live.limits {
                let (Some(used_percent), Some(window_duration_mins), Some(resets_at)) =
                    (limit.used_percent, limit.window_duration_mins, limit.resets_at)
                else {
                    continue;
                };
                let current = WindowObservation {
                    observed_at: now,
                    kind: limit.kind,
                    used_percent,
                    window_duration_mins,
                    resets_at,
                };
                let Some(earlier) = previous.get(kind_column(limit.kind)) else { continue };
                if let Some(event) = detect(*earlier, current) {
                    Self::insert_reset(&mut tx, provider_id, &event, "live", observed_at).await?;
                }
            }
        }
        sqlx::query(
            "UPDATE provider_instances SET plan_type = ?, earned_reset_count = ?, \
             last_live_success_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&live.plan_type)
        .bind(live.earned_reset_count.map(|value| value as i64))
        .bind(observed_at)
        .bind(observed_at)
        .bind(provider_id)
        .execute(&mut *tx)
        .await?;
        for limit in &live.limits {
            let kind = kind_column(limit.kind);
            sqlx::query(
                "INSERT INTO limit_current \
                 (provider_instance_id, window_kind, used_percent, window_duration_mins, resets_at, observed_at) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(provider_instance_id, window_kind) DO UPDATE SET \
                   used_percent = excluded.used_percent, window_duration_mins = excluded.window_duration_mins, \
                   resets_at = excluded.resets_at, observed_at = excluded.observed_at",
            )
            .bind(provider_id).bind(kind).bind(limit.used_percent).bind(limit.window_duration_mins)
            .bind(limit.resets_at).bind(observed_at).execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO limit_samples \
                 (provider_instance_id, window_kind, used_percent, window_duration_mins, resets_at, observed_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(provider_id).bind(kind).bind(limit.used_percent).bind(limit.window_duration_mins)
            .bind(limit.resets_at).bind(observed_at).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn load_current_observations(
        &self,
        provider_id: i64,
    ) -> Result<BTreeMap<String, WindowObservation>> {
        let rows = sqlx::query(
            "SELECT window_kind, used_percent, window_duration_mins, resets_at, observed_at \
             FROM limit_current WHERE provider_instance_id = ?",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        let mut observations = BTreeMap::new();
        for row in rows {
            let name: String = row.get("window_kind");
            let (Some(kind), Some(observed_at)) = (
                parse_kind(&name),
                row.try_get::<String, _>("observed_at").ok().as_deref().and_then(epoch_seconds),
            ) else {
                continue;
            };
            let (Some(used_percent), Some(window_duration_mins), Some(resets_at)) = (
                row.try_get::<Option<f64>, _>("used_percent")?,
                row.try_get::<Option<i64>, _>("window_duration_mins")?,
                row.try_get::<Option<i64>, _>("resets_at")?,
            ) else {
                continue;
            };
            observations.insert(
                name,
                WindowObservation { observed_at, kind, used_percent, window_duration_mins, resets_at },
            );
        }
        Ok(observations)
    }

    async fn insert_reset(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        event: &LimitResetEvent,
        source: &str,
        detected_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO limit_resets \
             (provider_instance_id, window_kind, window_duration_mins, anchored_at, new_resets_at, \
              previous_resets_at, used_percent_before, early_by_seconds, classification, source, detected_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id)
        .bind(kind_column(event.window_kind))
        .bind(event.window_duration_mins)
        .bind(event.anchored_at)
        .bind(event.new_resets_at)
        .bind(event.previous_resets_at)
        .bind(event.used_percent_before)
        .bind(event.early_by_seconds)
        .bind(event.classification.as_str())
        .bind(source)
        .bind(detected_at)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// The instant a rollout scan may start from, leaving enough overlap for a window
    /// that reset either side of the previous scan to still be paired with a reading.
    pub async fn reset_backfill_start(&self, provider: ProviderKind) -> Result<Option<i64>> {
        let last_completed: Option<String> = sqlx::query_scalar(
            "SELECT last_completed_at FROM retention_state WHERE job_name = ?",
        )
        .bind(backfill_job_name(provider))
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        Ok(last_completed
            .as_deref()
            .and_then(epoch_seconds)
            .map(|completed| completed - BACKFILL_OVERLAP_HOURS * 3_600))
    }

    /// Replays observations Codex logged itself, merged with the samples this machine
    /// already stored, so restarts that happened while QuotaStation was closed are still
    /// recorded. Storing an event twice is prevented by the table, not by the caller.
    pub async fn backfill_resets(
        &self,
        provider: ProviderKind,
        observations: &[WindowObservation],
        scanned_at: &str,
    ) -> Result<usize> {
        let provider_id = self.provider_id(provider).await?;
        let mut merged = observations.to_vec();
        merged.extend(self.load_sample_observations(provider_id).await?);
        merged.sort_by_key(|observation| observation.observed_at);

        let mut tracker = ResetTracker::default();
        let mut events = Vec::new();
        for observation in merged {
            if let Some(event) = tracker.push(observation) {
                events.push(event);
            }
        }

        let mut tx = self.pool.begin().await?;
        for event in &events {
            Self::insert_reset(&mut tx, provider_id, event, "backfill", scanned_at).await?;
        }
        sqlx::query(
            "INSERT INTO retention_state (job_name, last_completed_at, last_status, last_error) \
             VALUES (?, ?, 'succeeded', NULL) ON CONFLICT(job_name) DO UPDATE SET \
             last_completed_at=excluded.last_completed_at, last_status=excluded.last_status, last_error=NULL",
        )
        .bind(backfill_job_name(provider))
        .bind(scanned_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(events.len())
    }

    async fn load_sample_observations(&self, provider_id: i64) -> Result<Vec<WindowObservation>> {
        let rows = sqlx::query(
            "SELECT window_kind, used_percent, window_duration_mins, resets_at, observed_at \
             FROM limit_samples WHERE provider_instance_id = ? ORDER BY observed_at, id",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let kind = parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?;
                Some(WindowObservation {
                    observed_at: epoch_seconds(&row.try_get::<String, _>("observed_at").ok()?)?,
                    kind,
                    used_percent: row.try_get::<Option<f64>, _>("used_percent").ok()??,
                    window_duration_mins: row.try_get::<Option<i64>, _>("window_duration_mins").ok()??,
                    resets_at: row.try_get::<Option<i64>, _>("resets_at").ok()??,
                })
            })
            .collect())
    }

    pub async fn load_recent_resets(&self, provider: ProviderKind) -> Result<Vec<LimitResetEvent>> {
        let provider_id = self.provider_id(provider).await?;
        let rows = sqlx::query(
            "SELECT window_kind, window_duration_mins, anchored_at, new_resets_at, previous_resets_at, \
             used_percent_before, early_by_seconds, classification FROM limit_resets \
             WHERE provider_instance_id = ? ORDER BY anchored_at DESC LIMIT ?",
        )
        .bind(provider_id)
        .bind(RECENT_RESET_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let kind = parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?;
                let window_duration_mins: i64 = row.try_get("window_duration_mins").ok()?;
                let classification = match row.try_get::<String, _>("classification").ok()?.as_str() {
                    "unplanned" => ResetClassification::Unplanned,
                    _ => ResetClassification::Scheduled,
                };
                Some(LimitResetEvent {
                    window_kind: kind,
                    window_label: kind.window_label(Some(window_duration_mins)),
                    window_duration_mins,
                    anchored_at: row.try_get("anchored_at").ok()?,
                    new_resets_at: row.try_get("new_resets_at").ok()?,
                    previous_resets_at: row.try_get("previous_resets_at").ok()?,
                    used_percent_before: row.try_get("used_percent_before").ok()?,
                    early_by_seconds: row.try_get("early_by_seconds").ok()?,
                    classification,
                })
            })
            .collect())
    }

    pub async fn save_history(
        &self,
        provider: ProviderKind,
        history: &HistorySnapshot,
        observed_at: &str,
    ) -> Result<()> {
        let provider_id = self.provider_id(provider).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE provider_instances SET parser_revision = ?, last_history_success_at = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(CCUSAGE_REVISION).bind(observed_at).bind(observed_at).bind(provider_id)
        .execute(&mut *tx).await?;
        // Replace only the days the current parse covers. Sessions Codex has already
        // rotated away are absent from a parse, and their stored days must survive.
        for day in &history.days {
            sqlx::query("DELETE FROM daily_usage WHERE provider_instance_id = ? AND usage_date = ?")
                .bind(provider_id).bind(&day.date).execute(&mut *tx).await?;
            for row in &day.model_rows {
                self.insert_daily_model(&mut tx, provider_id, &day.date, row, observed_at).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    async fn insert_daily_model(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        date: &str,
        row: &DailyModelUsage,
        observed_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO daily_usage \
             (provider_instance_id, usage_date, model, service_tier, input_tokens, cache_read_tokens, \
              output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id).bind(date).bind(&row.model).bind(AGGREGATE_SERVICE_TIER).bind(row.input as i64)
        .bind(row.cache_read as i64).bind(row.output as i64).bind(row.reasoning as i64)
        .bind(row.total as i64).bind(row.cost_usd).bind(CCUSAGE_REVISION).bind(observed_at)
        .execute(&mut **tx).await?;
        Ok(())
    }

    pub async fn record_refresh(
        &self,
        provider: ProviderKind,
        acquisition_path: &str,
        started_at: &str,
        completed_at: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let provider_id = self.provider_id(provider).await?;
        sqlx::query(
            "INSERT INTO refresh_runs \
             (provider_instance_id, acquisition_path, started_at, completed_at, status, error_code, error_message) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id).bind(acquisition_path).bind(started_at).bind(completed_at)
        .bind(if error.is_some() { "failed" } else { "succeeded" })
        .bind(error.map(|_| "acquisition_failed")).bind(error)
        .execute(&self.pool).await?;
        Ok(())
    }

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

    async fn run_retention_at(&self, now: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Promote hourly data before its 60-day expiry. Reset segments remain separate.
        sqlx::query(
            "INSERT INTO limit_rollups (provider_instance_id, granularity, bucket_start, bucket_end, window_kind, \
               window_duration_mins, resets_at, reset_segment, start_used_percent, end_used_percent, min_used_percent, \
               max_used_percent, average_used_percent, sample_count) \
             SELECT h.provider_instance_id, 'daily', strftime('%Y-%m-%dT00:00:00Z', h.bucket_start), \
               strftime('%Y-%m-%dT23:59:59.999999999Z', h.bucket_start), h.window_kind, h.window_duration_mins, \
               h.resets_at, h.reset_segment, \
               (SELECT x.start_used_percent FROM limit_rollups x WHERE x.provider_instance_id = h.provider_instance_id \
                 AND x.granularity = 'hourly' AND date(x.bucket_start) = date(h.bucket_start) \
                 AND x.window_kind = h.window_kind AND x.reset_segment = h.reset_segment ORDER BY x.bucket_start LIMIT 1), \
               (SELECT x.end_used_percent FROM limit_rollups x WHERE x.provider_instance_id = h.provider_instance_id \
                 AND x.granularity = 'hourly' AND date(x.bucket_start) = date(h.bucket_start) \
                 AND x.window_kind = h.window_kind AND x.reset_segment = h.reset_segment ORDER BY x.bucket_start DESC LIMIT 1), \
               MIN(h.min_used_percent), MAX(h.max_used_percent), \
               SUM(h.average_used_percent * h.sample_count) / NULLIF(SUM(CASE WHEN h.average_used_percent IS NOT NULL THEN h.sample_count ELSE 0 END), 0), \
               SUM(h.sample_count) \
             FROM limit_rollups h WHERE h.granularity = 'hourly' AND datetime(h.bucket_start) < datetime(?, '-60 days') \
             GROUP BY h.provider_instance_id, date(h.bucket_start), h.window_kind, h.window_duration_mins, h.resets_at, h.reset_segment \
             ON CONFLICT(provider_instance_id, granularity, bucket_start, window_kind, reset_segment) DO UPDATE SET \
               bucket_end=excluded.bucket_end, window_duration_mins=excluded.window_duration_mins, resets_at=excluded.resets_at, \
               start_used_percent=excluded.start_used_percent, end_used_percent=excluded.end_used_percent, \
               min_used_percent=excluded.min_used_percent, max_used_percent=excluded.max_used_percent, \
               average_used_percent=excluded.average_used_percent, sample_count=excluded.sample_count",
        ).bind(now).execute(&mut *tx).await?;

        // Direct daily promotion handles samples that predate this migration.
        self.roll_up_samples(&mut tx, "daily", "%Y-%m-%dT00:00:00Z", "%Y-%m-%dT23:59:59.999999999Z", "-60 days", now).await?;
        self.roll_up_samples(&mut tx, "hourly", "%Y-%m-%dT%H:00:00Z", "%Y-%m-%dT%H:59:59.999999999Z", "-14 days", now).await?;

        sqlx::query("DELETE FROM limit_samples WHERE datetime(observed_at) < datetime(?, '-14 days')").bind(now).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM limit_rollups WHERE granularity = 'hourly' AND datetime(bucket_start) < datetime(?, '-60 days')").bind(now).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM limit_rollups WHERE granularity = 'daily' AND datetime(bucket_start) < datetime(?, '-180 days')").bind(now).execute(&mut *tx).await?;
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
             SELECT s.provider_instance_id, ?, strftime('{bucket_start}', s.observed_at), strftime('{bucket_end}', s.observed_at), \
               s.window_kind, s.window_duration_mins, s.resets_at, COALESCE(CAST(s.resets_at AS TEXT), 'none'), \
               (SELECT x.used_percent FROM limit_samples x WHERE x.provider_instance_id=s.provider_instance_id \
                 AND strftime('{bucket_start}', x.observed_at)=strftime('{bucket_start}', s.observed_at) AND x.window_kind=s.window_kind \
                 AND x.window_duration_mins IS s.window_duration_mins AND x.resets_at IS s.resets_at ORDER BY x.observed_at, x.id LIMIT 1), \
               (SELECT x.used_percent FROM limit_samples x WHERE x.provider_instance_id=s.provider_instance_id \
                 AND strftime('{bucket_start}', x.observed_at)=strftime('{bucket_start}', s.observed_at) AND x.window_kind=s.window_kind \
                 AND x.window_duration_mins IS s.window_duration_mins AND x.resets_at IS s.resets_at ORDER BY x.observed_at DESC, x.id DESC LIMIT 1), \
               MIN(s.used_percent), MAX(s.used_percent), AVG(s.used_percent), COUNT(*) FROM limit_samples s \
             WHERE datetime(s.observed_at) < datetime(?, '{cutoff}') AND datetime(s.observed_at) >= datetime(?, '-180 days') \
             GROUP BY s.provider_instance_id, strftime('{bucket_start}', s.observed_at), s.window_kind, s.window_duration_mins, s.resets_at \
             ON CONFLICT(provider_instance_id, granularity, bucket_start, window_kind, reset_segment) DO UPDATE SET \
               bucket_end=excluded.bucket_end, window_duration_mins=excluded.window_duration_mins, resets_at=excluded.resets_at, \
               start_used_percent=excluded.start_used_percent, end_used_percent=excluded.end_used_percent, min_used_percent=excluded.min_used_percent, \
               max_used_percent=excluded.max_used_percent, average_used_percent=excluded.average_used_percent, sample_count=excluded.sample_count"
        );
        sqlx::query(&query).bind(granularity).bind(now).bind(now).execute(&mut **tx).await?;
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
            None => RetentionDiagnostics { status: "pending".into(), last_completed_at: None, error: None },
        })
    }

    pub async fn load_snapshot(&self, provider: ProviderKind) -> Result<ProviderSnapshot> {
        let provider_id = self.provider_id(provider).await?;
        let instance = sqlx::query(
            "SELECT plan_type, earned_reset_count, last_live_success_at, last_history_success_at \
             FROM provider_instances WHERE id = ?",
        ).bind(provider_id).fetch_one(&self.pool).await?;
        let mut snapshot = ProviderSnapshot::new(provider);
        snapshot.plan_type = instance.try_get("plan_type")?;
        snapshot.earned_reset_count = instance.try_get::<Option<i64>, _>("earned_reset_count")?.map(|v| v as u64);
        let live_success: Option<String> = instance.try_get("last_live_success_at")?;
        let history_success: Option<String> = instance.try_get("last_history_success_at")?;
        snapshot.last_success_at = [live_success, history_success].into_iter().flatten().max();

        let limits = sqlx::query(
            "SELECT window_kind, used_percent, window_duration_mins, resets_at \
             FROM limit_current WHERE provider_instance_id = ? ORDER BY window_kind",
        ).bind(provider_id).fetch_all(&self.pool).await?;
        snapshot.limits = limits.into_iter().filter_map(|row| {
            let kind = parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?;
            let window_duration_mins = row.try_get("window_duration_mins").ok();
            Some(LimitWindow {
                kind,
                label: kind.window_label(window_duration_mins),
                used_percent: row.try_get("used_percent").ok(),
                remaining_percent: row.try_get::<Option<f64>, _>("used_percent").ok().flatten().map(|used| (100.0 - used).clamp(0.0, 100.0)),
                window_duration_mins,
                resets_at: row.try_get("resets_at").ok(),
            })
        }).collect();

        snapshot.recent_resets = self.load_recent_resets(provider).await?;
        let date = jiff::Zoned::now().date().to_string();
        let today = self.load_usage_range(provider, &date, &date).await?;
        snapshot.today = today.usage;
        snapshot.models = today.models;
        snapshot.api_equivalent_cost_usd = today.api_equivalent_cost_usd;
        snapshot.freshness = if snapshot.last_success_at.is_some() { Freshness::Stale } else { Freshness::Unavailable };
        snapshot.update_compact_status();
        snapshot.pricing_catalog_revision = PRICING_CATALOG_REVISION.to_string();
        Ok(snapshot)
    }

    pub async fn load_usage_range(
        &self,
        provider: ProviderKind,
        start_date: &str,
        end_date: &str,
    ) -> Result<UsageRangeSnapshot> {
        let start = jiff::civil::Date::from_str(start_date).context("invalid start date")?;
        let end = jiff::civil::Date::from_str(end_date).context("invalid end date")?;
        anyhow::ensure!(start <= end, "start date must not be after end date");

        let provider_id = self.provider_id(provider).await?;
        let rows = sqlx::query(
            "SELECT usage_date, model, SUM(input_tokens) AS input_tokens, \
             SUM(cache_read_tokens) AS cache_read_tokens, SUM(output_tokens) AS output_tokens, \
             SUM(reasoning_tokens) AS reasoning_tokens, SUM(total_tokens) AS total_tokens, \
             SUM(COALESCE(estimated_cost_usd, 0)) AS estimated_cost_usd \
             FROM daily_usage WHERE provider_instance_id = ? AND usage_date BETWEEN ? AND ? \
             GROUP BY usage_date, model ORDER BY usage_date ASC, total_tokens DESC",
        )
        .bind(provider_id).bind(start.to_string()).bind(end.to_string())
        .fetch_all(&self.pool).await?;

        let mut total = TokenUsage::default();
        let mut total_cost = 0.0;
        let mut model_totals: BTreeMap<String, u64> = BTreeMap::new();
        let mut day_totals: BTreeMap<String, (TokenUsage, f64)> = BTreeMap::new();
        for row in rows {
            let date: String = row.get("usage_date");
            let model: String = row.get("model");
            let usage = TokenUsage {
                input: row.get::<i64, _>("input_tokens") as u64,
                cache_read: row.get::<i64, _>("cache_read_tokens") as u64,
                output: row.get::<i64, _>("output_tokens") as u64,
                reasoning: row.get::<i64, _>("reasoning_tokens") as u64,
                total: row.get::<i64, _>("total_tokens") as u64,
            };
            let cost = row.get::<f64, _>("estimated_cost_usd");
            total.input += usage.input;
            total.cache_read += usage.cache_read;
            total.output += usage.output;
            total.reasoning += usage.reasoning;
            total.total += usage.total;
            total_cost += cost;
            *model_totals.entry(model).or_default() += usage.total;
            let day = day_totals.entry(date).or_insert_with(|| (TokenUsage::default(), 0.0));
            day.0.input += usage.input;
            day.0.cache_read += usage.cache_read;
            day.0.output += usage.output;
            day.0.reasoning += usage.reasoning;
            day.0.total += usage.total;
            day.1 += cost;
        }

        let mut models: Vec<_> = model_totals.into_iter().map(|(model, tokens)| ModelUsage {
            model,
            tokens,
            percent: if total.total == 0 { 0.0 } else { tokens as f64 / total.total as f64 * 100.0 },
        }).collect();
        models.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        let days = day_totals.into_iter().map(|(date, (usage, cost))| DailyUsagePoint {
            date,
            api_equivalent_cost_usd: (usage.total > 0).then_some(cost),
            usage,
        }).collect();

        Ok(UsageRangeSnapshot {
            start_date: start.to_string(),
            end_date: end.to_string(),
            api_equivalent_cost_usd: (total.total > 0).then_some(total_cost),
            usage: total,
            models,
            days,
        })
    }

    #[cfg(test)]
    async fn table_names(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name <> '_sqlx_migrations' ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn load_acquisition_diagnostics(
        &self,
        provider: ProviderKind,
    ) -> Result<Vec<AcquisitionDiagnostics>> {
        let provider_id = self.provider_id(provider).await?;
        let instance = sqlx::query(
            "SELECT last_live_success_at, last_history_success_at FROM provider_instances WHERE id = ?",
        )
        .bind(provider_id)
        .fetch_one(&self.pool)
        .await?;
        let live_success: Option<String> = instance.try_get("last_live_success_at")?;
        let history_success: Option<String> = instance.try_get("last_history_success_at")?;
        let rows = sqlx::query(
            "SELECT acquisition_path, started_at, status, error_message FROM refresh_runs \
             WHERE provider_instance_id = ? AND id IN (\
               SELECT MAX(id) FROM refresh_runs WHERE provider_instance_id = ? GROUP BY acquisition_path\
             )",
        )
        .bind(provider_id)
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;

        let mut latest = BTreeMap::new();
        for row in rows {
            latest.insert(
                row.get::<String, _>("acquisition_path"),
                (
                    row.get::<String, _>("started_at"),
                    row.get::<String, _>("status"),
                    row.try_get::<Option<String>, _>("error_message")?,
                ),
            );
        }

        let name = provider.display_name();
        let mut paths = vec![
            (provider.live_path(), format!("{name} live quota"), live_success),
            (provider.history_path(), format!("{name} local history"), history_success),
        ];
        // A provider with a second, optional quota source reports it separately, so a
        // cross-check that could not run is visible without looking like a failed read.
        if let Some(path) = provider.cross_check_path() {
            let last_success_at = latest
                .get(&path)
                .filter(|(_, status, _)| status == "succeeded")
                .map(|(started_at, _, _)| started_at.clone());
            paths.push((path, format!("{name} online cross-check"), last_success_at));
        }
        Ok(paths
        .into_iter()
        .map(|(path, label, last_success_at)| {
            let run = latest.get(&path);
            AcquisitionDiagnostics {
                acquisition_path: path,
                label,
                status: run.map(|(_, status, _)| status.clone()).unwrap_or_else(|| "pending".to_string()),
                last_attempt_at: run.map(|(started_at, _, _)| started_at.clone()),
                last_success_at,
                error: run.and_then(|(_, _, error)| error.clone()),
            }
        })
        .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    const CODEX: ProviderKind = ProviderKind::Codex;
    use crate::domain::{CrossCheck, HistoryDay, LimitWindow, LiveSnapshot};

    /// Each test owns a database file in the temporary directory and removes it, along
    /// with the write-ahead files SQLite may leave beside it, when it finishes.
    struct TempDatabase {
        path: std::path::PathBuf,
    }

    impl TempDatabase {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let name = format!(
                "quotastation-{}-{}.db",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            Self { path: std::env::temp_dir().join(name) }
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut path = self.path.clone().into_os_string();
                path.push(suffix);
                let _ = std::fs::remove_file(path);
            }
        }
    }

    async fn open_storage() -> (Storage, TempDatabase) {
        let database = TempDatabase::new();
        let storage = Storage::open(&database.path).await.expect("open storage");
        (storage, database)
    }

    fn day(date: &str, model: &str, total: u64) -> HistoryDay {
        HistoryDay {
            date: date.to_string(),
            usage: TokenUsage { input: total / 2, cache_read: 0, output: total / 2, reasoning: 0, total },
            models: vec![ModelUsage { model: model.to_string(), tokens: total, percent: 100.0 }],
            cost_usd: 1.5,
            model_rows: vec![DailyModelUsage {
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

    #[tokio::test]
    async fn migrations_leave_only_the_tables_the_core_writes() {
        let (storage, _database) = open_storage().await;
        assert_eq!(
            storage.table_names().await.expect("read table names"),
            [
                "daily_usage",
                "limit_current",
                "limit_resets",
                "limit_rollups",
                "limit_samples",
                "provider_instances",
                "refresh_runs",
                "retention_state",
            ]
        );
    }

    #[tokio::test]
    async fn a_history_refresh_replaces_only_the_days_it_parsed() {
        let (storage, _database) = open_storage().await;
        let first = HistorySnapshot { days: vec![day("2026-08-01", "gpt-5", 100), day("2026-08-02", "gpt-5", 200)] };
        storage.save_history(CODEX, &first, "2026-08-02T00:00:00Z").await.expect("save first history");

        // A later parse no longer sees the rotated-away session that produced 08-01.
        let second = HistorySnapshot { days: vec![day("2026-08-02", "gpt-5", 500)] };
        storage.save_history(CODEX, &second, "2026-08-02T01:00:00Z").await.expect("save second history");

        let range = storage.load_usage_range(CODEX, "2026-08-01", "2026-08-02").await.expect("load range");
        assert_eq!(range.days.len(), 2, "the day outside the parse must survive");
        assert_eq!(range.days[0].usage.total, 100);
        assert_eq!(range.days[1].usage.total, 500, "the reparsed day must be replaced, not added to");
        assert_eq!(range.usage.total, 600);
    }

    #[tokio::test]
    async fn a_usage_range_reports_only_the_requested_days() {
        let (storage, _database) = open_storage().await;
        let history = HistorySnapshot {
            days: vec![day("2026-08-01", "gpt-5", 100), day("2026-08-02", "gpt-5-codex", 300)],
        };
        storage.save_history(CODEX, &history, "2026-08-02T00:00:00Z").await.expect("save history");

        let range = storage.load_usage_range(CODEX, "2026-08-02", "2026-08-02").await.expect("load range");
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
            limits: vec![LimitWindow {
                kind: LimitKind::Primary,
                label: LimitKind::Primary.window_label(Some(300)),
                used_percent: Some(40.0),
                remaining_percent: Some(60.0),
                window_duration_mins: Some(300),
                resets_at: Some(1_800_000_000),
            }],
            cross_check: CrossCheck::NotAttempted,
        };
        storage.save_live(CODEX, &live, "2026-08-11T00:00:00Z").await.expect("save live");

        let snapshot = storage.load_snapshot(CODEX).await.expect("load snapshot");
        assert_eq!(snapshot.plan_type.as_deref(), Some("plus"));
        assert_eq!(snapshot.limits.len(), 1);
        assert_eq!(snapshot.limits[0].label, "5-hour window");
        assert_eq!(snapshot.limits[0].remaining_percent, Some(60.0));
        assert_eq!(snapshot.freshness, Freshness::Stale, "a restored snapshot is never fresh");
    }

    const WEEK_MINUTES: i64 = 10_080;

    fn weekly_live(used_percent: f64, resets_at: i64) -> LiveSnapshot {
        LiveSnapshot {
            plan_type: Some("plus".to_string()),
            earned_reset_count: Some(0),
            limits: vec![LimitWindow {
                kind: LimitKind::Primary,
                label: LimitKind::Primary.window_label(Some(WEEK_MINUTES)),
                used_percent: Some(used_percent),
                remaining_percent: Some(100.0 - used_percent),
                window_duration_mins: Some(WEEK_MINUTES),
                resets_at: Some(resets_at),
            }],
            cross_check: CrossCheck::NotAttempted,
        }
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
        storage.save_live(CODEX, &weekly_live(15.0, expiry), "2026-06-04T10:00:00Z").await.expect("save first");
        storage.save_live(CODEX, &weekly_live(4.0, expiry + 7_200), "2026-06-04T15:00:00Z").await.expect("save second");
        assert!(storage.load_recent_resets(CODEX).await.expect("load resets").is_empty());
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
        storage.backfill_resets(CODEX, &observations, "2026-08-12T01:00:00Z").await.expect("rerun the backfill");
        let events = storage.load_recent_resets(CODEX).await.expect("load resets");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].classification, ResetClassification::Unplanned);
        assert!(storage.reset_backfill_start(CODEX).await.expect("read cursor").is_some());
    }
}
