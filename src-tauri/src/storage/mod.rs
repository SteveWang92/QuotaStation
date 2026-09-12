//! Everything QuotaStation has measured, in one SQLite database beside its settings.
//!
//! This holds the connection, the identity of a provider inside it, and the reads that
//! belong to no one area. The four areas are files of their own: what the quota readings
//! leave behind, the token history, how long either is kept, and what another device
//! contributed.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::domain::{
    AcquisitionDiagnostics, Freshness, LimitKind, LimitWindow, ModelUsage,
    PRICING_CATALOG_REVISION, PaceLevel, ProviderSnapshot, QuotaLevel, TokenUsage, WindowSource,
};
use crate::providers::ProviderKind;

mod import;
mod resets;
mod retention;
mod usage;

pub use import::{DeviceImport, DeviceRecord};

/// The Codex daily report aggregates every service tier into one row per model, so the
/// tier dimension of `daily_usage` records that the row spans tiers rather than guessing
/// one. A per-tier writer must use the tier it observed, never this value.
const AGGREGATE_SERVICE_TIER: &str = "mixed";

/// The device every row this machine parsed is stored under. A machine reports itself to
/// other machines by the identifier in its settings file, but stores its own work under
/// this fixed name: the rows outlive any identifier, and a migration that had to know one
/// could not read the settings file to find it.
pub const LOCAL_DEVICE: &str = "local";

/// How long quota readings are kept at the granularity they arrived at, before they become
/// the daily summaries that replace them. This is the window an unexplained reset can still
/// be diagnosed in, and ninety days of readings cost single-digit megabytes; a shorter
/// window loses the samples behind a restart before anyone asks about it.
const SAMPLE_HISTORY_DAYS: i64 = 90;

/// What joins a session's model names in the one column that holds them. A model name
/// never contains it, and nothing queries the column, so the list is stored as it reads.
const MODEL_SEPARATOR: &str = ", ";

/// Which of the two usage tables a device split is being read from, and the column its
/// buckets are keyed by. A calendar range reads the daily rows; a rolling window of hours
/// can only be answered by the hourly ones, because the days at its ends are partial.
#[derive(Debug, Clone, Copy)]
enum BucketTable {
    Daily,
    Hourly,
}

impl BucketTable {
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::Daily => ("daily_usage", "usage_date"),
            Self::Hourly => ("hourly_usage", "hour_start"),
        }
    }
}

/// One bucket of usage being accumulated — a day or an hour — as its rows arrive one
/// model at a time.
#[derive(Default)]
struct Bucket {
    usage: TokenUsage,
    cost: f64,
    models: BTreeMap<String, u64>,
}

impl Bucket {
    fn add(&mut self, usage: &TokenUsage, cost: f64, model: &str) {
        add_usage(&mut self.usage, usage);
        self.cost += cost;
        *self.models.entry(model.to_string()).or_default() += usage.total;
    }

    /// The bucket as a surface reads it: a cost only where something was spent, and the
    /// models ranked by their share of this bucket alone.
    fn finish(self) -> (TokenUsage, Option<f64>, Vec<ModelUsage>) {
        let cost = (self.usage.total > 0).then_some(self.cost);
        let models = rank_models(self.models, self.usage.total);
        (self.usage, cost, models)
    }
}

fn add_usage(total: &mut TokenUsage, usage: &TokenUsage) {
    total.input += usage.input;
    total.cache_read += usage.cache_read;
    total.output += usage.output;
    total.reasoning += usage.reasoning;
    total.total += usage.total;
}

/// Turns a model's token totals into the share-of-total form every surface draws, largest
/// first. The same shape describes a range and a single day inside it, so both are built
/// here rather than each computing percentages of a different denominator.
fn rank_models(totals: BTreeMap<String, u64>, denominator: u64) -> Vec<ModelUsage> {
    let mut models: Vec<_> = totals
        .into_iter()
        .map(|(model, tokens)| ModelUsage {
            model,
            tokens,
            percent: if denominator == 0 {
                0.0
            } else {
                tokens as f64 / denominator as f64 * 100.0
            },
        })
        .collect();
    models.sort_by_key(|model| std::cmp::Reverse(model.tokens));
    models
}

fn epoch_seconds(value: &str) -> Option<i64> {
    value
        .parse::<i64>()
        .ok()
        .or_else(|| value.parse::<jiff::Timestamp>().ok().map(|timestamp| timestamp.as_second()))
}

/// The stored spelling of a quota window kind, as every area that reads one needs it.
fn parse_kind(value: &str) -> Option<LimitKind> {
    match value {
        "primary" => Some(LimitKind::Primary),
        "secondary" => Some(LimitKind::Secondary),
        _ => None,
    }
}

/// How long a session's two cost figures are kept. They are read again from the logs while
/// those still exist, so the rows outlast the sessions behind them by exactly as long as a
/// quota reading is kept, which is the span any question about a period can still be asked
/// over.
const SESSION_COST_HISTORY_DAYS: i64 = SAMPLE_HISTORY_DAYS;

#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
}

impl Storage {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create application data directory")?;
        }
        let options =
            SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    async fn provider_id(&self, provider: ProviderKind) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT id FROM provider_instances WHERE provider = ?")
            .bind(provider.key())
            .fetch_one(&self.pool)
            .await?)
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

    pub async fn load_snapshot(&self, provider: ProviderKind) -> Result<ProviderSnapshot> {
        let provider_id = self.provider_id(provider).await?;
        let instance = sqlx::query(
            "SELECT plan_type, earned_reset_count, earned_reset_expires_at, \
             last_live_success_at, last_history_success_at \
             FROM provider_instances WHERE id = ?",
        )
        .bind(provider_id)
        .fetch_one(&self.pool)
        .await?;
        let mut snapshot = ProviderSnapshot::new(provider);
        snapshot.plan_type = instance.try_get("plan_type")?;
        snapshot.earned_reset_count =
            instance.try_get::<Option<i64>, _>("earned_reset_count")?.map(|v| v as u64);
        snapshot.earned_reset_expires_at = instance.try_get("earned_reset_expires_at")?;
        let live_success: Option<String> = instance.try_get("last_live_success_at")?;
        let history_success: Option<String> = instance.try_get("last_history_success_at")?;
        snapshot.last_live_success_at = live_success;
        snapshot.last_history_success_at = history_success;

        let limits = sqlx::query(
            "SELECT window_kind, used_percent, window_duration_mins, resets_at, observed_at, source \
             FROM limit_current WHERE provider_instance_id = ? ORDER BY window_kind",
        ).bind(provider_id).fetch_all(&self.pool).await?;
        snapshot.limits = limits
            .into_iter()
            .filter_map(|row| {
                let kind = parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?;
                // Each of these three is nullable, and SQLite hands a null column to a decode
                // that did not ask for an option as a zero rather than as an error. Read them as
                // options, or a window nothing has measured comes back reading 0% used and
                // restarting at the epoch.
                let window_duration_mins =
                    row.try_get::<Option<i64>, _>("window_duration_mins").ok()?;
                Some(LimitWindow {
                    kind,
                    label: kind.window_label(window_duration_mins),
                    used_percent: row.try_get::<Option<f64>, _>("used_percent").ok()?,
                    window_duration_mins,
                    resets_at: row.try_get::<Option<i64>, _>("resets_at").ok()?,
                    source: WindowSource::parse(&row.try_get::<String, _>("source").ok()?)?,
                    observed_at: epoch_seconds(&row.try_get::<String, _>("observed_at").ok()?)?,
                    freshness: Freshness::Stale,
                    status_level: QuotaLevel::Healthy,
                    pace: PaceLevel::OnTrack,
                })
            })
            .collect();

        snapshot.recent_resets = self.load_recent_resets(provider).await?;
        let date = jiff::Zoned::now().date().to_string();
        let today = self.load_usage_range(Some(provider), None, &date, &date).await?;
        snapshot.today = today.usage;
        snapshot.models = today.models;
        snapshot.api_equivalent_cost_usd = today.api_equivalent_cost_usd;
        snapshot.resolve_derived_state();
        snapshot.pricing_catalog_revision = PRICING_CATALOG_REVISION.to_string();
        Ok(snapshot)
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
             WHERE provider_instance_id = ? AND id IN ( \
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
        let paths = vec![
            (provider.live_path(), format!("{name} live quota"), live_success),
            (provider.history_path(), format!("{name} local history"), history_success),
        ];
        Ok(paths
            .into_iter()
            .map(|(path, label, last_success_at)| {
                let run = latest.get(&path);
                AcquisitionDiagnostics {
                    acquisition_path: path,
                    label,
                    status: run
                        .map(|(_, status, _)| status.clone())
                        .unwrap_or_else(|| "pending".to_string()),
                    last_attempt_at: run.map(|(started_at, _, _)| started_at.clone()),
                    last_success_at,
                    error: run.and_then(|(_, _, error)| error.clone()),
                }
            })
            .collect())
    }
}

/// A throwaway database for a test, shared with the other modules whose tests need one.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::Storage;

    /// Each test owns a database file in the temporary directory and removes it, along
    /// with the write-ahead files SQLite may leave beside it, when it finishes.
    pub(crate) struct TempDatabase {
        pub(crate) path: std::path::PathBuf,
        directory: Option<std::path::PathBuf>,
    }

    impl TempDatabase {
        pub(crate) fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let name = format!(
                "quotastation-{}-{}.db",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            Self { path: std::env::temp_dir().join(name), directory: None }
        }

        pub(crate) fn in_unicode_directory() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "QuotaStation 数据 {} {}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            Self { path: directory.join("quota station.db"), directory: Some(directory) }
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut path = self.path.clone().into_os_string();
                path.push(suffix);
                let _ = std::fs::remove_file(path);
            }
            if let Some(directory) = &self.directory {
                let _ = std::fs::remove_dir(directory);
            }
        }
    }

    pub(crate) async fn open_storage() -> (Storage, TempDatabase) {
        let database = TempDatabase::new();
        let storage = Storage::open(&database.path).await.expect("open storage");
        (storage, database)
    }
}

#[cfg(test)]
mod tests;
