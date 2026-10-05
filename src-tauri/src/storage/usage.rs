//! The token history: what each day, hour and session cost, written as the parsers report
//! it and read back for whatever range the dashboard asks for.

use std::{collections::BTreeMap, str::FromStr};

use anyhow::{Context, Result};
use sqlx::{AssertSqlSafe, Row};

use crate::domain::{
    CCUSAGE_REVISION, DailyUsagePoint, DeviceUsage, HistorySnapshot, HourlyUsagePoint,
    ModelUsageRow, PRICING_CATALOG_REVISION, SessionCost, SessionCostSnapshot, SessionModelCost,
    TokenUsage, UsageHoursSnapshot, UsageRangeSnapshot, UsageWindowSnapshot,
};
use crate::providers::ProviderKind;

use super::{
    AGGREGATE_SERVICE_TIER, Bucket, BucketTable, LOCAL_DEVICE, MODEL_SEPARATOR, Storage, add_usage,
    rank_models,
};

/// The cost a parsed row is stored at. A price is settled once a row has one: a row whose
/// tokens are unchanged keeps the cost it was first priced at, whatever the catalog now
/// says, and only a row that was never priced, or whose tokens changed, takes the new one.
fn kept_cost(kept: &BTreeMap<String, (u64, f64)>, row: &ModelUsageRow) -> f64 {
    match kept.get(&row.model) {
        Some(&(total, cost)) if total == row.total && cost > 0.0 => cost,
        _ => row.cost_usd,
    }
}

/// The part of a stored session the next parse needs in order to keep its settled prices.
struct StoredSession {
    total_tokens: u64,
    computed_cost_usd: f64,
    /// `None` on a row written before sessions recorded what each of their models cost.
    model_costs: Option<Vec<SessionModelCost>>,
    pricing_catalog_revision: String,
}

/// What a session is stored at, by the rule `kept_cost` applies to a day, applied to each
/// of its models. A row that recorded only its total keeps that total whole while the
/// session's tokens are unchanged, its models' shares scaled to match.
///
/// The flag says the stored total was kept whole, which is when it keeps the revision of
/// the catalog that priced it too. That total is returned as stored rather than summed
/// again, so an unchanged session reads back the same figure on every refresh.
fn settle_session(
    session: &SessionCost,
    stored: Option<&StoredSession>,
) -> (f64, Vec<SessionModelCost>, bool) {
    let mut cost = session.computed_cost_usd;
    let mut models = session.model_costs.clone();
    let Some(stored) = stored else { return (cost, models, false) };
    let unchanged = stored.total_tokens == session.usage.total;
    match &stored.model_costs {
        Some(stored_models) => {
            let mut every_model_settled = true;
            for model in &mut models {
                let settled = stored_models.iter().find(|kept| {
                    kept.model == model.model && kept.tokens == model.tokens && kept.cost_usd > 0.0
                });
                match settled {
                    Some(settled) => {
                        cost += settled.cost_usd - model.cost_usd;
                        model.cost_usd = settled.cost_usd;
                    }
                    None => every_model_settled = false,
                }
            }
            if every_model_settled && unchanged {
                return (stored.computed_cost_usd, models, true);
            }
        }
        None if unchanged && stored.computed_cost_usd > 0.0 => {
            if cost > 0.0 {
                let scale = stored.computed_cost_usd / cost;
                for model in &mut models {
                    model.cost_usd *= scale;
                }
            }
            return (stored.computed_cost_usd, models, true);
        }
        None => {}
    }
    (cost, models, false)
}

/// One stored session comparison, as the surfaces read it back.
fn session_cost_from_row(row: &sqlx::sqlite::SqliteRow) -> SessionCost {
    let tokens = |column: &str| row.get::<i64, _>(column).max(0) as u64;
    let models: String = row.get("models");
    SessionCost {
        session_id: row.get("session_id"),
        session_started_at: row.get("session_started_at"),
        duration_ms: row.get("duration_ms"),
        computed_cost_usd: row.get("computed_cost_usd"),
        independent: row.get("independent"),
        reported_cost_usd: row.get("reported_cost_usd"),
        reported_complete: row.get("reported_complete"),
        api_duration_ms: row.get("api_duration_ms"),
        lines_added: row.get("lines_added"),
        lines_removed: row.get("lines_removed"),
        usage: TokenUsage {
            input: tokens("input_tokens"),
            cache_read: tokens("cache_read_tokens"),
            output: tokens("output_tokens"),
            reasoning: tokens("reasoning_tokens"),
            total: tokens("total_tokens"),
        },
        models: models
            .split(MODEL_SEPARATOR)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
            .collect(),
        model_costs: Vec::new(),
    }
}

impl Storage {
    pub async fn save_history(
        &self,
        provider: ProviderKind,
        history: &HistorySnapshot,
        aggregation_timezone: &str,
        observed_at: &str,
    ) -> Result<()> {
        let provider_id = self.provider_id(provider).await?;
        let mut tx = self.pool.begin().await?;
        let previous_timezone: Option<String> =
            sqlx::query_scalar("SELECT aggregation_timezone FROM provider_instances WHERE id = ?")
                .bind(provider_id)
                .fetch_one(&mut *tx)
                .await?;
        if previous_timezone.as_deref().is_some_and(|previous| previous != aggregation_timezone) {
            // Every other device's rows go: an imported row is keyed by the local hour it was
            // aggregated in, and this machine has just changed which hours those are.
            // Forgetting where each device's file stood is what brings the others back — the
            // next refresh reads every one of them again and re-checks it against the zone now
            // in force.
            for table in ["daily_usage", "hourly_usage"] {
                // The table name comes from the literal list above, and every value is bound.
                sqlx::query(AssertSqlSafe(format!(
                    "DELETE FROM {table} WHERE provider_instance_id = ? AND device <> ?"
                )))
                .bind(provider_id)
                .bind(LOCAL_DEVICE)
                .execute(&mut *tx)
                .await?;
            }
            // A forgotten device keeps its marker: a new zone is not its file changing.
            sqlx::query("UPDATE devices SET source_modified_at = NULL WHERE forgotten = 0")
                .execute(&mut *tx)
                .await?;
            // This machine's rows are rebuilt from its logs as far back as the logs still
            // reach, which is the earliest day this parse produced. The hourly window is far
            // shorter than any log is kept, so all of it is rebuilt. A day before the logs
            // reach — Claude Code deletes old transcripts — has nothing left to rebuild it
            // from, so it keeps the date it was filed under rather than being lost; only the
            // hours either side of its midnight can sit on the neighbouring day.
            sqlx::query("DELETE FROM hourly_usage WHERE provider_instance_id = ? AND device = ?")
                .bind(provider_id)
                .bind(LOCAL_DEVICE)
                .execute(&mut *tx)
                .await?;
            if let Some(first) = history.days.iter().map(|day| day.date.as_str()).min() {
                sqlx::query(
                    "DELETE FROM daily_usage WHERE provider_instance_id = ? AND device = ? \
                     AND usage_date >= ?",
                )
                .bind(provider_id)
                .bind(LOCAL_DEVICE)
                .bind(first)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query(
            "UPDATE provider_instances SET parser_revision = ?, aggregation_timezone = ?, \
             last_history_success_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(CCUSAGE_REVISION)
        .bind(aggregation_timezone)
        .bind(observed_at)
        .bind(observed_at)
        .bind(provider_id)
        .execute(&mut *tx)
        .await?;
        // Replace only the days the current parse covers, and only this machine's rows in
        // them: a parse of these logs says nothing about what another machine did that day.
        // Sessions Codex has already rotated away are absent from a parse, and their stored
        // days must survive.
        for day in &history.days {
            let kept =
                Self::kept_costs(&mut tx, provider_id, "daily_usage", "usage_date", &day.date)
                    .await?;
            sqlx::query(
                "DELETE FROM daily_usage WHERE provider_instance_id = ? AND device = ? \
                 AND usage_date = ?",
            )
            .bind(provider_id)
            .bind(LOCAL_DEVICE)
            .bind(&day.date)
            .execute(&mut *tx)
            .await?;
            for row in &day.model_rows {
                Self::insert_daily_model(
                    &mut tx,
                    provider_id,
                    LOCAL_DEVICE,
                    &day.date,
                    row,
                    kept_cost(&kept, row),
                    observed_at,
                )
                .await?;
            }
        }
        // The hourly rows are replaced the same way, and only ever cover the recent window
        // the parser produces them for; retention removes the ones that fall out of it.
        for hour in &history.hours {
            let kept = Self::kept_costs(
                &mut tx,
                provider_id,
                "hourly_usage",
                "hour_start",
                &hour.hour_start,
            )
            .await?;
            sqlx::query(
                "DELETE FROM hourly_usage WHERE provider_instance_id = ? AND device = ? \
                 AND hour_start = ?",
            )
            .bind(provider_id)
            .bind(LOCAL_DEVICE)
            .bind(&hour.hour_start)
            .execute(&mut *tx)
            .await?;
            for row in &hour.model_rows {
                Self::insert_hourly_model(
                    &mut tx,
                    provider_id,
                    LOCAL_DEVICE,
                    &hour.hour_start,
                    row,
                    kept_cost(&kept, row),
                    observed_at,
                )
                .await?;
            }
        }
        // The hours a recorded restart's window spans have just been rewritten, so the
        // totals built from them are rebuilt here rather than left a parse behind.
        Self::refresh_reset_tokens(&mut tx, provider_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// What this machine's stored rows for one day or hour cost, by model, with the tokens
    /// each was priced for. Read before the rows are replaced, so a price already settled
    /// survives the parse that rewrites them.
    async fn kept_costs(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        table: &str,
        key_column: &str,
        key: &str,
    ) -> Result<BTreeMap<String, (u64, f64)>> {
        // Both names come from the literals at the call sites, and every value is bound.
        let rows: Vec<(String, i64, Option<f64>)> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT model, total_tokens, estimated_cost_usd FROM {table}              WHERE provider_instance_id = ? AND device = ? AND {key_column} = ?"
        )))
        .bind(provider_id)
        .bind(LOCAL_DEVICE)
        .bind(key)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(model, total, cost)| Some((model, (total.max(0) as u64, cost?))))
            .collect())
    }

    async fn insert_daily_model(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        device: &str,
        date: &str,
        row: &ModelUsageRow,
        cost_usd: f64,
        observed_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO daily_usage \
             (provider_instance_id, device, usage_date, model, service_tier, input_tokens, cache_read_tokens, \
              output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id).bind(device).bind(date).bind(&row.model).bind(AGGREGATE_SERVICE_TIER).bind(row.input as i64)
        .bind(row.cache_read as i64).bind(row.output as i64).bind(row.reasoning as i64)
        .bind(row.total as i64).bind(cost_usd).bind(CCUSAGE_REVISION).bind(observed_at)
        .execute(&mut **tx).await?;
        Ok(())
    }

    async fn insert_hourly_model(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        device: &str,
        hour_start: &str,
        row: &ModelUsageRow,
        cost_usd: f64,
        observed_at: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO hourly_usage              (provider_instance_id, device, hour_start, model, service_tier, input_tokens, cache_read_tokens,               output_tokens, reasoning_tokens, total_tokens, estimated_cost_usd, parser_revision, updated_at)              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id).bind(device).bind(hour_start).bind(&row.model).bind(AGGREGATE_SERVICE_TIER)
        .bind(row.input as i64).bind(row.cache_read as i64).bind(row.output as i64)
        .bind(row.reasoning as i64).bind(row.total as i64).bind(cost_usd)
        .bind(CCUSAGE_REVISION).bind(observed_at)
        .execute(&mut **tx).await?;
        Ok(())
    }

    /// Stores each session the parser read, priced from the catalog, with the client's own
    /// figures where it recorded any. A session is written once and then corrected, since
    /// a session logged today is summarised again once more work goes through it. A row
    /// stays after the client deletes the log behind it, because nothing else records it.
    pub async fn save_session_costs(
        &self,
        provider: ProviderKind,
        sessions: &[SessionCost],
        observed_at: &str,
    ) -> Result<()> {
        let provider_id = self.provider_id(provider).await?;
        let mut tx = self.pool.begin().await?;
        let rows: Vec<(String, i64, f64, Option<String>, String)> = sqlx::query_as(
            "SELECT session_id, total_tokens, computed_cost_usd, model_costs, \
             pricing_catalog_revision FROM session_costs WHERE provider_instance_id = ?",
        )
        .bind(provider_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut stored = BTreeMap::new();
        for (session_id, total_tokens, computed_cost_usd, model_costs, revision) in rows {
            let model_costs = model_costs
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .context("a stored session's model costs could not be read")?;
            stored.insert(
                session_id,
                StoredSession {
                    total_tokens: total_tokens.max(0) as u64,
                    computed_cost_usd,
                    model_costs,
                    pricing_catalog_revision: revision,
                },
            );
        }
        for session in sessions {
            let previous = stored.get(&session.session_id);
            let (computed_cost_usd, model_costs, kept) = settle_session(session, previous);
            let revision = match previous {
                Some(previous) if kept => previous.pricing_catalog_revision.as_str(),
                _ => PRICING_CATALOG_REVISION,
            };
            sqlx::query(
                "INSERT INTO session_costs \
                 (provider_instance_id, session_id, session_started_at, duration_ms, \
                  computed_cost_usd, independent, reported_cost_usd, reported_complete, \
                  api_duration_ms, lines_added, lines_removed, input_tokens, cache_read_tokens, \
                  output_tokens, reasoning_tokens, total_tokens, models, model_costs, \
                  parser_revision, pricing_catalog_revision, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(provider_instance_id, session_id) DO UPDATE SET \
                 session_started_at=excluded.session_started_at, \
                 duration_ms=excluded.duration_ms, \
                 computed_cost_usd=excluded.computed_cost_usd, \
                 independent=excluded.independent, \
                 reported_cost_usd=excluded.reported_cost_usd, \
                 reported_complete=excluded.reported_complete, \
                 api_duration_ms=excluded.api_duration_ms, \
                 lines_added=excluded.lines_added, \
                 lines_removed=excluded.lines_removed, \
                 input_tokens=excluded.input_tokens, \
                 cache_read_tokens=excluded.cache_read_tokens, \
                 output_tokens=excluded.output_tokens, \
                 reasoning_tokens=excluded.reasoning_tokens, \
                 total_tokens=excluded.total_tokens, \
                 models=excluded.models, \
                 model_costs=excluded.model_costs, \
                 parser_revision=excluded.parser_revision, \
                 pricing_catalog_revision=excluded.pricing_catalog_revision, \
                 updated_at=excluded.updated_at",
            )
            .bind(provider_id)
            .bind(&session.session_id)
            .bind(&session.session_started_at)
            .bind(session.duration_ms)
            .bind(computed_cost_usd)
            .bind(session.independent)
            .bind(session.reported_cost_usd)
            .bind(session.reported_complete)
            .bind(session.api_duration_ms)
            .bind(session.lines_added)
            .bind(session.lines_removed)
            .bind(session.usage.input as i64)
            .bind(session.usage.cache_read as i64)
            .bind(session.usage.output as i64)
            .bind(session.usage.reasoning as i64)
            .bind(session.usage.total as i64)
            .bind(session.models.join(MODEL_SEPARATOR))
            .bind(serde_json::to_string(&model_costs)?)
            .bind(CCUSAGE_REVISION)
            .bind(revision)
            .bind(observed_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every session that started inside a range, newest first, with the costs of the
    /// comparable ones summed. `None` is the combined view, as it is for usage.
    ///
    /// The range is read in the application zone's days like every other history query,
    /// because the dates on screen are the ones the reader picked in that zone.
    pub async fn session_costs(
        &self,
        provider: Option<ProviderKind>,
        start_date: &str,
        end_date: &str,
    ) -> Result<SessionCostSnapshot> {
        let provider_id = match provider {
            Some(kind) => Some(self.provider_id(kind).await?),
            None => None,
        };
        let from = crate::clock::day_start(jiff::civil::Date::from_str(start_date)?)?;
        let until = crate::clock::day_start(jiff::civil::Date::from_str(end_date)?.tomorrow()?)?;
        let rows = sqlx::query(
            "SELECT session_id, session_started_at, duration_ms, computed_cost_usd, \
             independent, reported_cost_usd, reported_complete, api_duration_ms, lines_added, \
             lines_removed, input_tokens, cache_read_tokens, output_tokens, reasoning_tokens, \
             total_tokens, models \
             FROM session_costs \
             WHERE (? IS NULL OR provider_instance_id = ?) \
             AND unixepoch(session_started_at) >= ? AND unixepoch(session_started_at) < ? \
             ORDER BY session_started_at DESC",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(from)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;
        let sessions: Vec<SessionCost> = rows.iter().map(session_cost_from_row).collect();
        // Only the sessions the client also priced belong in either sum: adding every
        // computed cost to a reported total that covers a few of them would state a gap
        // that measures which sessions carry the record, not how the two sides differ.
        let compared = sessions.iter().filter(|session| session.reported_cost_usd.is_some());
        let reported_cost_usd =
            compared.clone().filter_map(|session| session.reported_cost_usd).sum();
        let computed_cost_usd = compared.map(|session| session.computed_cost_usd).sum();
        Ok(SessionCostSnapshot { sessions, reported_cost_usd, computed_cost_usd })
    }

    /// Providers that have usage rows on any device. Imported rows count exactly like
    /// local ones here: this list drives which histories have a tab, not which clients this
    /// machine can ask for live quota.
    pub async fn load_usage_providers(&self) -> Result<Vec<ProviderKind>> {
        let keys: Vec<String> = sqlx::query_scalar(
            "SELECT provider FROM provider_instances WHERE id IN ( \
             SELECT provider_instance_id FROM daily_usage UNION \
             SELECT provider_instance_id FROM hourly_usage)",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(ProviderKind::ALL
            .into_iter()
            .filter(|provider| keys.iter().any(|key| key == provider.key()))
            .collect())
    }

    /// The first day carrying usage for the selected provider and device filters.
    pub async fn load_usage_start_date(
        &self,
        provider: Option<ProviderKind>,
        device: Option<&str>,
    ) -> Result<Option<String>> {
        let provider_id = match provider {
            Some(kind) => Some(self.provider_id(kind).await?),
            None => None,
        };
        Ok(sqlx::query_scalar(
            "SELECT MIN(usage_date) FROM daily_usage \
             WHERE (? IS NULL OR provider_instance_id = ?) AND (? IS NULL OR device = ?)",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(device)
        .bind(device)
        .fetch_one(&self.pool)
        .await?)
    }

    /// The usage in a date range, for one provider or for every provider at once.
    ///
    /// `None` is the combined view: the rows of every provider instance are counted
    /// together, which is what the totals, the per-day stack and the model ranking are
    /// summed from. Nothing about that sum is provider-specific, so it is one query with
    /// the filter dropped rather than a second read path.
    pub async fn load_usage_range(
        &self,
        provider: Option<ProviderKind>,
        device: Option<&str>,
        start_date: &str,
        end_date: &str,
    ) -> Result<UsageRangeSnapshot> {
        let start = jiff::civil::Date::from_str(start_date).context("invalid start date")?;
        let end = jiff::civil::Date::from_str(end_date).context("invalid end date")?;
        anyhow::ensure!(start <= end, "start date must not be after end date");

        let provider_id = match provider {
            Some(kind) => Some(self.provider_id(kind).await?),
            None => None,
        };
        let rows = sqlx::query(
            "SELECT usage_date, model, SUM(input_tokens) AS input_tokens, \
             SUM(cache_read_tokens) AS cache_read_tokens, SUM(output_tokens) AS output_tokens, \
             SUM(reasoning_tokens) AS reasoning_tokens, SUM(total_tokens) AS total_tokens, \
             SUM(COALESCE(estimated_cost_usd, 0)) AS estimated_cost_usd \
             FROM daily_usage WHERE (? IS NULL OR provider_instance_id = ?) \
             AND (? IS NULL OR device = ?) \
             AND usage_date BETWEEN ? AND ? \
             GROUP BY usage_date, model ORDER BY usage_date ASC, total_tokens DESC",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(device)
        .bind(device)
        .bind(start.to_string())
        .bind(end.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut total = TokenUsage::default();
        let mut total_cost = 0.0;
        let mut model_totals: BTreeMap<String, u64> = BTreeMap::new();
        let mut day_totals: BTreeMap<String, (TokenUsage, f64, BTreeMap<String, u64>)> =
            BTreeMap::new();
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
            *model_totals.entry(model.clone()).or_default() += usage.total;
            let day = day_totals
                .entry(date)
                .or_insert_with(|| (TokenUsage::default(), 0.0, BTreeMap::new()));
            day.0.input += usage.input;
            day.0.cache_read += usage.cache_read;
            day.0.output += usage.output;
            day.0.reasoning += usage.reasoning;
            day.0.total += usage.total;
            day.1 += cost;
            *day.2.entry(model).or_default() += usage.total;
        }

        let models = rank_models(model_totals, total.total);
        let days = day_totals
            .into_iter()
            .map(|(date, (usage, cost, day_models))| DailyUsagePoint {
                date,
                api_equivalent_cost_usd: (usage.total > 0).then_some(cost),
                models: rank_models(day_models, usage.total),
                usage,
            })
            .collect();

        let split_total = if device.is_none() {
            total.total
        } else {
            self.load_usage_total(
                provider_id,
                BucketTable::Daily,
                &start.to_string(),
                &end.to_string(),
            )
            .await?
        };
        let devices = self
            .load_device_split(
                provider_id,
                BucketTable::Daily,
                &start.to_string(),
                &end.to_string(),
                split_total,
            )
            .await?;
        Ok(UsageRangeSnapshot {
            start_date: start.to_string(),
            end_date: end.to_string(),
            api_equivalent_cost_usd: (total.total > 0).then_some(total_cost),
            usage: total,
            models,
            days,
            devices,
        })
    }

    async fn load_usage_total(
        &self,
        provider_id: Option<i64>,
        bucket: BucketTable,
        start: &str,
        end: &str,
    ) -> Result<u64> {
        let (table, column) = bucket.parts();
        // `BucketTable::parts` yields only fixed table and column names, and every value is bound.
        let total: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT COALESCE(SUM(total_tokens), 0) FROM {table} \
             WHERE (? IS NULL OR provider_instance_id = ?) AND {column} BETWEEN ? AND ?"
        )))
        .bind(provider_id)
        .bind(provider_id)
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await?;
        Ok(total as u64)
    }

    /// Which machines the range's tokens came from. Devices with nothing in the range are
    /// left out: a machine that was switched off all week is not a nought-token row.
    async fn load_device_split(
        &self,
        provider_id: Option<i64>,
        bucket: BucketTable,
        start: &str,
        end: &str,
        total: u64,
    ) -> Result<Vec<DeviceUsage>> {
        let (table, column) = bucket.parts();
        // `BucketTable::parts` yields only fixed table and column names, and every value is bound.
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {table}.device AS device, devices.display_name AS display_name, \
             SUM(total_tokens) AS tokens FROM {table} \
             JOIN devices ON devices.id = {table}.device \
             WHERE (? IS NULL OR provider_instance_id = ?) AND {column} BETWEEN ? AND ? \
             GROUP BY {table}.device HAVING tokens > 0 ORDER BY tokens DESC"
        )))
        .bind(provider_id)
        .bind(provider_id)
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let tokens = row.get::<i64, _>("tokens") as u64;
                let device_id: String = row.get("device");
                DeviceUsage {
                    local: device_id == LOCAL_DEVICE,
                    device_id,
                    display_name: row.get("display_name"),
                    tokens,
                    percent: if total == 0 { 0.0 } else { tokens as f64 / total as f64 * 100.0 },
                }
            })
            .collect())
    }

    /// The same range at hourly resolution, for the short ranges that are drawn hour by
    /// hour.
    ///
    /// Only the hours the parser has rows for come back — an hour nothing ran in is an
    /// absence, and the renderer draws the empty buckets from the range itself. Hours
    /// older than the retention window simply do not exist here; the caller is expected
    /// to have chosen a range short enough that they cannot be asked for.
    pub async fn load_usage_hours(
        &self,
        provider: Option<ProviderKind>,
        device: Option<&str>,
        start_date: &str,
        end_date: &str,
    ) -> Result<UsageHoursSnapshot> {
        let start = jiff::civil::Date::from_str(start_date).context("invalid start date")?;
        let end = jiff::civil::Date::from_str(end_date).context("invalid end date")?;
        anyhow::ensure!(start <= end, "start date must not be after end date");

        let provider_id = match provider {
            Some(kind) => Some(self.provider_id(kind).await?),
            None => None,
        };
        let rows = sqlx::query(
            "SELECT hour_start, model, SUM(input_tokens) AS input_tokens,              SUM(cache_read_tokens) AS cache_read_tokens, SUM(output_tokens) AS output_tokens,              SUM(reasoning_tokens) AS reasoning_tokens, SUM(total_tokens) AS total_tokens,              SUM(COALESCE(estimated_cost_usd, 0)) AS estimated_cost_usd              FROM hourly_usage WHERE (? IS NULL OR provider_instance_id = ?)              AND (? IS NULL OR device = ?) AND date(hour_start) BETWEEN ? AND ?              GROUP BY hour_start, model ORDER BY hour_start ASC, total_tokens DESC",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(device)
        .bind(device)
        .bind(start.to_string())
        .bind(end.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut hour_totals: BTreeMap<String, (TokenUsage, f64, BTreeMap<String, u64>)> =
            BTreeMap::new();
        for row in rows {
            let hour_start: String = row.get("hour_start");
            let model: String = row.get("model");
            let usage = TokenUsage {
                input: row.get::<i64, _>("input_tokens") as u64,
                cache_read: row.get::<i64, _>("cache_read_tokens") as u64,
                output: row.get::<i64, _>("output_tokens") as u64,
                reasoning: row.get::<i64, _>("reasoning_tokens") as u64,
                total: row.get::<i64, _>("total_tokens") as u64,
            };
            let hour = hour_totals
                .entry(hour_start)
                .or_insert_with(|| (TokenUsage::default(), 0.0, BTreeMap::new()));
            hour.0.input += usage.input;
            hour.0.cache_read += usage.cache_read;
            hour.0.output += usage.output;
            hour.0.reasoning += usage.reasoning;
            hour.0.total += usage.total;
            hour.1 += row.get::<f64, _>("estimated_cost_usd");
            *hour.2.entry(model).or_default() += usage.total;
        }

        Ok(UsageHoursSnapshot {
            start_date: start.to_string(),
            end_date: end.to_string(),
            hours: hour_totals
                .into_iter()
                .map(|(hour_start, (usage, cost, hour_models))| HourlyUsagePoint {
                    hour_start,
                    api_equivalent_cost_usd: (usage.total > 0).then_some(cost),
                    models: rank_models(hour_models, usage.total),
                    usage,
                })
                .collect(),
        })
    }

    /// A rolling window of hours, answered as the totals and the hours behind them.
    ///
    /// A calendar range is read from `daily_usage`, which cannot express "the last
    /// twenty-four hours": both days such a window touches are partial. Everything here
    /// is summed from `hourly_usage` instead — the headline totals, the day rows, the
    /// model mix and the device split — so no figure on the surface describes different
    /// hours than the chart beside it. `hourly_usage` is kept for
    /// [`HOURLY_HISTORY_DAYS`](crate::domain::HOURLY_HISTORY_DAYS) days, which is how far
    /// back a window may reach.
    pub async fn load_usage_window(
        &self,
        provider: Option<ProviderKind>,
        device: Option<&str>,
        start_hour: &str,
        end_hour: &str,
    ) -> Result<UsageWindowSnapshot> {
        anyhow::ensure!(start_hour <= end_hour, "start hour must not be after end hour");
        let provider_id = match provider {
            Some(kind) => Some(self.provider_id(kind).await?),
            None => None,
        };
        let rows = sqlx::query(
            "SELECT hour_start, model, SUM(input_tokens) AS input_tokens, \
             SUM(cache_read_tokens) AS cache_read_tokens, SUM(output_tokens) AS output_tokens, \
             SUM(reasoning_tokens) AS reasoning_tokens, SUM(total_tokens) AS total_tokens, \
             SUM(COALESCE(estimated_cost_usd, 0)) AS estimated_cost_usd \
             FROM hourly_usage WHERE (? IS NULL OR provider_instance_id = ?) \
             AND (? IS NULL OR device = ?) AND hour_start BETWEEN ? AND ? \
             GROUP BY hour_start, model ORDER BY hour_start ASC, total_tokens DESC",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(device)
        .bind(device)
        .bind(start_hour)
        .bind(end_hour)
        .fetch_all(&self.pool)
        .await?;

        let mut total = TokenUsage::default();
        let mut total_cost = 0.0;
        let mut model_totals: BTreeMap<String, u64> = BTreeMap::new();
        let mut hour_totals: BTreeMap<String, Bucket> = BTreeMap::new();
        let mut day_totals: BTreeMap<String, Bucket> = BTreeMap::new();
        for row in rows {
            let hour_start: String = row.get("hour_start");
            let model: String = row.get("model");
            let usage = TokenUsage {
                input: row.get::<i64, _>("input_tokens") as u64,
                cache_read: row.get::<i64, _>("cache_read_tokens") as u64,
                output: row.get::<i64, _>("output_tokens") as u64,
                reasoning: row.get::<i64, _>("reasoning_tokens") as u64,
                total: row.get::<i64, _>("total_tokens") as u64,
            };
            let cost = row.get::<f64, _>("estimated_cost_usd");
            add_usage(&mut total, &usage);
            total_cost += cost;
            *model_totals.entry(model.clone()).or_default() += usage.total;
            // The hour key opens with the local date it belongs to, which is the day row
            // this hour is counted into: a partial day is still that day.
            day_totals.entry(hour_start[..10].to_string()).or_default().add(&usage, cost, &model);
            hour_totals.entry(hour_start).or_default().add(&usage, cost, &model);
        }

        let split_total = if device.is_none() {
            total.total
        } else {
            self.load_usage_total(provider_id, BucketTable::Hourly, start_hour, end_hour).await?
        };
        let devices = self
            .load_device_split(provider_id, BucketTable::Hourly, start_hour, end_hour, split_total)
            .await?;
        let (start_date, end_date) = (start_hour[..10].to_string(), end_hour[..10].to_string());
        Ok(UsageWindowSnapshot {
            range: UsageRangeSnapshot {
                start_date: start_date.clone(),
                end_date: end_date.clone(),
                api_equivalent_cost_usd: (total.total > 0).then_some(total_cost),
                models: rank_models(model_totals, total.total),
                days: day_totals
                    .into_iter()
                    .map(|(date, bucket)| {
                        let (usage, api_equivalent_cost_usd, models) = bucket.finish();
                        DailyUsagePoint { date, usage, api_equivalent_cost_usd, models }
                    })
                    .collect(),
                usage: total,
                devices,
            },
            hours: UsageHoursSnapshot {
                start_date,
                end_date,
                hours: hour_totals
                    .into_iter()
                    .map(|(hour_start, bucket)| {
                        let (usage, api_equivalent_cost_usd, models) = bucket.finish();
                        HourlyUsagePoint { hour_start, usage, api_equivalent_cost_usd, models }
                    })
                    .collect(),
            },
        })
    }
}
