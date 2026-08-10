use std::{collections::BTreeMap, path::Path, str::FromStr};

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

use crate::domain::{
    AcquisitionDiagnostics, CCUSAGE_REVISION, DailyModelUsage, DailyUsagePoint, Freshness,
    HistorySnapshot, LimitKind, LimitWindow, LiveSnapshot, ModelUsage, PRICING_CATALOG_REVISION,
    ProviderSnapshot, RetentionDiagnostics, TokenUsage, UsageRangeSnapshot,
};

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

    async fn provider_id(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT id FROM provider_instances WHERE provider = 'codex'")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn save_live(&self, live: &LiveSnapshot, observed_at: &str) -> Result<()> {
        let provider_id = self.provider_id().await?;
        let mut tx = self.pool.begin().await?;
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
            let kind = match limit.kind { LimitKind::Primary => "primary", LimitKind::Secondary => "secondary" };
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

    pub async fn save_history(&self, history: &HistorySnapshot, observed_at: &str) -> Result<()> {
        let provider_id = self.provider_id().await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE provider_instances SET parser_revision = ?, last_history_success_at = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(CCUSAGE_REVISION).bind(observed_at).bind(observed_at).bind(provider_id)
        .execute(&mut *tx).await?;
        sqlx::query("DELETE FROM daily_usage WHERE provider_instance_id = ?")
            .bind(provider_id).execute(&mut *tx).await?;
        for day in &history.days {
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
             VALUES (?, ?, ?, 'mixed', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id).bind(date).bind(&row.model).bind(row.input as i64)
        .bind(row.cache_read as i64).bind(row.output as i64).bind(row.reasoning as i64)
        .bind(row.total as i64).bind(row.cost_usd).bind(CCUSAGE_REVISION).bind(observed_at)
        .execute(&mut **tx).await?;
        Ok(())
    }

    pub async fn record_refresh(
        &self,
        acquisition_path: &str,
        started_at: &str,
        completed_at: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let provider_id = self.provider_id().await?;
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
            let message = sanitize_storage_error(&error.to_string());
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

    pub async fn load_snapshot(&self) -> Result<ProviderSnapshot> {
        let provider_id = self.provider_id().await?;
        let provider = sqlx::query(
            "SELECT plan_type, earned_reset_count, last_live_success_at, last_history_success_at \
             FROM provider_instances WHERE id = ?",
        ).bind(provider_id).fetch_one(&self.pool).await?;
        let mut snapshot = ProviderSnapshot::default();
        snapshot.plan_type = provider.try_get("plan_type")?;
        snapshot.earned_reset_count = provider.try_get::<Option<i64>, _>("earned_reset_count")?.map(|v| v as u64);
        let live_success: Option<String> = provider.try_get("last_live_success_at")?;
        let history_success: Option<String> = provider.try_get("last_history_success_at")?;
        snapshot.last_success_at = [live_success, history_success].into_iter().flatten().max();

        let limits = sqlx::query(
            "SELECT window_kind, used_percent, window_duration_mins, resets_at \
             FROM limit_current WHERE provider_instance_id = ? ORDER BY window_kind",
        ).bind(provider_id).fetch_all(&self.pool).await?;
        snapshot.limits = limits.into_iter().filter_map(|row| {
            let kind: String = row.try_get("window_kind").ok()?;
            let kind = match kind.as_str() { "primary" => LimitKind::Primary, "secondary" => LimitKind::Secondary, _ => return None };
            Some(LimitWindow {
                kind,
                label: match kind { LimitKind::Primary => "Primary window", LimitKind::Secondary => "Secondary window" }.to_string(),
                used_percent: row.try_get("used_percent").ok(),
                remaining_percent: row.try_get::<Option<f64>, _>("used_percent").ok().flatten().map(|used| (100.0 - used).clamp(0.0, 100.0)),
                window_duration_mins: row.try_get("window_duration_mins").ok(),
                resets_at: row.try_get("resets_at").ok(),
            })
        }).collect();

        let date = jiff::Zoned::now().date().to_string();
        let today = self.load_usage_range(&date, &date).await?;
        snapshot.today = today.usage;
        snapshot.models = today.models;
        snapshot.api_equivalent_cost_usd = today.api_equivalent_cost_usd;
        snapshot.freshness = if snapshot.last_success_at.is_some() { Freshness::Stale } else { Freshness::Unavailable };
        snapshot.update_compact_status();
        snapshot.pricing_catalog_revision = PRICING_CATALOG_REVISION.to_string();
        Ok(snapshot)
    }

    pub async fn load_usage_range(&self, start_date: &str, end_date: &str) -> Result<UsageRangeSnapshot> {
        let start = jiff::civil::Date::from_str(start_date).context("invalid start date")?;
        let end = jiff::civil::Date::from_str(end_date).context("invalid end date")?;
        anyhow::ensure!(start <= end, "start date must not be after end date");

        let provider_id = self.provider_id().await?;
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

    pub async fn load_acquisition_diagnostics(&self) -> Result<Vec<AcquisitionDiagnostics>> {
        let provider_id = self.provider_id().await?;
        let provider = sqlx::query(
            "SELECT last_live_success_at, last_history_success_at FROM provider_instances WHERE id = ?",
        )
        .bind(provider_id)
        .fetch_one(&self.pool)
        .await?;
        let live_success: Option<String> = provider.try_get("last_live_success_at")?;
        let history_success: Option<String> = provider.try_get("last_history_success_at")?;
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

        Ok([
            ("codex_live", "Live quota", live_success),
            ("codex_history", "Local history", history_success),
        ]
        .into_iter()
        .map(|(path, label, last_success_at)| {
            let run = latest.get(path);
            AcquisitionDiagnostics {
                acquisition_path: path.to_string(),
                label: label.to_string(),
                status: run.map(|(_, status, _)| status.clone()).unwrap_or_else(|| "pending".to_string()),
                last_attempt_at: run.map(|(started_at, _, _)| started_at.clone()),
                last_success_at,
                error: run.and_then(|(_, _, error)| error.clone()),
            }
        })
        .collect())
    }
}

fn sanitize_storage_error(error: &str) -> String {
    let line = error.lines().next().unwrap_or("Retention failed");
    if line.len() > 220 { format!("{}…", &line[..220]) } else { line.to_string() }
}
