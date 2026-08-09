use std::{path::Path, str::FromStr};

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

use crate::domain::{
    CCUSAGE_REVISION, DailyModelUsage, Freshness, HistorySnapshot, LimitKind, LimitWindow,
    LiveSnapshot, ModelUsage, PRICING_CATALOG_REVISION, ProviderSnapshot, TokenUsage,
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
        sqlx::query("DELETE FROM daily_usage WHERE provider_instance_id = ? AND usage_date = ?")
            .bind(provider_id).bind(&history.date).execute(&mut *tx).await?;
        for row in &history.model_rows {
            self.insert_daily_model(&mut tx, provider_id, &history.date, row, observed_at).await?;
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
                window_duration_mins: row.try_get("window_duration_mins").ok(),
                resets_at: row.try_get("resets_at").ok(),
            })
        }).collect();

        let date = jiff::Zoned::now().date().to_string();
        let rows = sqlx::query(
            "SELECT model, input_tokens, cache_read_tokens, output_tokens, reasoning_tokens, \
             total_tokens, estimated_cost_usd FROM daily_usage \
             WHERE provider_instance_id = ? AND usage_date = ?",
        ).bind(provider_id).bind(&date).fetch_all(&self.pool).await?;
        let mut models = Vec::new();
        let mut total = TokenUsage::default();
        let mut cost = 0.0;
        for row in rows {
            let row_total = row.get::<i64, _>("total_tokens") as u64;
            total.input += row.get::<i64, _>("input_tokens") as u64;
            total.cache_read += row.get::<i64, _>("cache_read_tokens") as u64;
            total.output += row.get::<i64, _>("output_tokens") as u64;
            total.reasoning += row.get::<i64, _>("reasoning_tokens") as u64;
            total.total += row_total;
            cost += row.get::<Option<f64>, _>("estimated_cost_usd").unwrap_or(0.0);
            models.push((row.get::<String, _>("model"), row_total));
        }
        snapshot.models = models.into_iter().map(|(model, tokens)| ModelUsage {
            model,
            tokens,
            percent: if total.total == 0 { 0.0 } else { tokens as f64 / total.total as f64 * 100.0 },
        }).collect();
        snapshot.models.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        snapshot.today = total;
        snapshot.api_equivalent_cost_usd = (snapshot.today.total > 0).then_some(cost);
        snapshot.freshness = if snapshot.last_success_at.is_some() { Freshness::Stale } else { Freshness::Unavailable };
        snapshot.pricing_catalog_revision = PRICING_CATALOG_REVISION.to_string();
        Ok(snapshot)
    }
}
