//! What the quota readings leave behind: each reading as it is taken, the window it
//! belongs to, and the restart the tracker infers from the pair.
//!
//! `crate::resets` decides that a window restarted; this stores the decision and reads it
//! back.

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use anyhow::{Context, Result};
use sqlx::Row;

use crate::domain::{
    HOURLY_HISTORY_DAYS, LimitKind, LimitResetEvent, LiveSnapshot, QuotaHistoryPoint,
    QuotaHistorySnapshot, QuotaHistoryWindow, ResetClassification, WindowSource,
};
use crate::providers::ProviderKind;
use crate::resets::{ResetTracker, WindowObservation, detect};

use super::{Storage, epoch_seconds, parse_kind};

/// The rollout scan skips files older than its previous run, with this much overlap so a
/// window that reset across the boundary still has an earlier reading to compare against.
const BACKFILL_OVERLAP_HOURS: i64 = 48;

/// How many restarts the surfaces are given. They annotate the window running now and
/// list the ones before it, neither of which needs the whole history.
const RECENT_RESET_LIMIT: i64 = 8;

/// The columns every query over `limit_resets` selects. The token total among them is
/// stored rather than summed on the way out, so it outlives the hourly rows it was built
/// from — see [`Storage::refresh_reset_tokens`].
const RESET_COLUMNS: &str = "window_kind, window_duration_mins, anchored_at, new_resets_at, previous_resets_at, \
    used_percent_before, tokens_in_window, early_by_seconds, classification";

/// The restart before this one of the same window, which is where the window this one
/// closed began. `NULL` for the first restart recorded of a window. A window is identified
/// by its duration rather than its slot, because Codex moves one between primary and
/// secondary: matching on the slot pairs a weekly restart with a five-hour one and shortens
/// the window it closed to a few hours.
const RESET_PREVIOUS_ANCHOR: &str = "( \
      SELECT previous.anchored_at FROM limit_resets AS previous \
      WHERE previous.provider_instance_id = limit_resets.provider_instance_id \
      AND previous.window_duration_mins = limit_resets.window_duration_mins \
      AND previous.anchored_at < limit_resets.anchored_at \
      ORDER BY previous.anchored_at DESC LIMIT 1)";

/// When the window a restart closed stopped counting. A window that expired unused stops
/// at its published expiry and the next one is anchored at the next request, which can be
/// hours of idleness later; a window rebuilt early stops when the restart anchored the new
/// one. Reading the earlier of the two keeps that idle gap out of the closed window.
const RESET_WINDOW_END: &str = "MIN(limit_resets.previous_resets_at, limit_resets.anchored_at)";

pub(super) fn kind_column(kind: LimitKind) -> &'static str {
    match kind {
        LimitKind::Primary => "primary",
        LimitKind::Secondary => "secondary",
    }
}

/// One recorded restart, from a row carrying [`RESET_COLUMNS`]. The recent list and the
/// range query select the same columns, so they read them the same way.
fn reset_event(row: sqlx::sqlite::SqliteRow) -> Option<LimitResetEvent> {
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
        tokens_in_window: row
            .try_get::<Option<i64>, _>("tokens_in_window")
            .ok()
            .flatten()
            .and_then(|tokens| u64::try_from(tokens).ok()),
        early_by_seconds: row.try_get("early_by_seconds").ok()?,
        classification,
    })
}

/// Each provider replays its own logs, so the scan watermarks cannot share a row.
fn backfill_job_name(provider: ProviderKind) -> String {
    format!("{}_reset_backfill", provider.key())
}

impl Storage {
    pub async fn save_live(
        &self,
        provider: ProviderKind,
        live: &LiveSnapshot,
        observed_at: &str,
    ) -> Result<()> {
        let provider_id = self.provider_id(provider).await?;
        // A restart is inferred only from the source that publishes the window rather than
        // deriving it — see `ProviderKind::authoritative_window_source`. Comparing a reading
        // against one that measured the same window a different way manufactures restarts.
        let authoritative = provider.authoritative_window_source();
        let previous = self.load_current_observations(provider_id, authoritative).await?;
        let measured = self.windows_with_an_allowance(provider_id).await?;
        let mut tx = self.pool.begin().await?;
        if let Some(now) = epoch_seconds(observed_at) {
            for limit in &live.limits {
                if limit.source != authoritative {
                    continue;
                }
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
                // Paired by duration rather than by slot: Codex moves a window between
                // `primary` and `secondary`, and the reading a restart is recognised
                // against belongs to the window, not to the name it arrived under.
                let Some(earlier) = previous.get(&window_duration_mins) else { continue };
                if let Some(event) = detect(*earlier, current) {
                    Self::insert_reset(&mut tx, provider_id, &event, "live", observed_at).await?;
                }
            }
        }
        sqlx::query(
            "UPDATE provider_instances SET plan_type = ?, earned_reset_count = ?, \
             earned_reset_expires_at = ?, last_live_success_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&live.plan_type)
        .bind(live.earned_reset_count.map(|value| value as i64))
        .bind(live.earned_reset_expires_at)
        .bind(observed_at)
        .bind(observed_at)
        .bind(provider_id)
        .execute(&mut *tx)
        .await?;
        for limit in &live.limits {
            let kind = kind_column(limit.kind);
            // A window that arrives without an allowance is a weaker reading of a window
            // already measured, not news about it: Claude Code publishes the five-hour window
            // with a restart but no percentage while one closes, and the session-log fallback
            // beneath it never carries a percentage at all. Such a reading is ignored whole —
            // stored, it would empty every surface and discard the reading a restart is
            // recognised against, so that restart would go unrecorded. The log still records
            // what arrived.
            if limit.used_percent.is_none() && measured.contains(kind) {
                continue;
            }
            sqlx::query(
                "INSERT INTO limit_current \
                 (provider_instance_id, window_kind, used_percent, window_duration_mins, resets_at, observed_at, source) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(provider_instance_id, window_kind) DO UPDATE SET \
                   used_percent = excluded.used_percent, window_duration_mins = excluded.window_duration_mins, \
                   resets_at = excluded.resets_at, observed_at = excluded.observed_at, source = excluded.source",
            )
            .bind(provider_id).bind(kind).bind(limit.used_percent).bind(limit.window_duration_mins)
            .bind(limit.resets_at).bind(limit.observed_at.to_string()).bind(limit.source.as_str())
            .execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO limit_samples \
                 (provider_instance_id, window_kind, used_percent, window_duration_mins, resets_at, observed_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            // The sample is dated by the reading, not by the refresh that carried it. A
            // cached status-line reading is republished every couple of minutes for as long
            // as it stays usable, and dating those by the refresh would draw one afternoon's
            // share as every following day's peak.
            .bind(provider_id).bind(kind).bind(limit.used_percent).bind(limit.window_duration_mins)
            .bind(limit.resets_at)
            .bind(
                jiff::Timestamp::from_second(limit.observed_at)
                    .map(|reading| reading.to_string())
                    .unwrap_or_else(|_| observed_at.to_string()),
            )
            .execute(&mut *tx).await?;
        }
        Self::refresh_reset_tokens(&mut tx, provider_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The windows a percentage has already been measured for. A reading that has none
    /// cannot replace one of these; see the guard in [`Storage::save_live`].
    async fn windows_with_an_allowance(&self, provider_id: i64) -> Result<BTreeSet<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT window_kind FROM limit_current \
             WHERE provider_instance_id = ? AND used_percent IS NOT NULL",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    async fn load_current_observations(
        &self,
        provider_id: i64,
        source: WindowSource,
    ) -> Result<BTreeMap<i64, WindowObservation>> {
        let rows = sqlx::query(
            "SELECT window_kind, used_percent, window_duration_mins, resets_at, observed_at \
             FROM limit_current WHERE provider_instance_id = ? AND source = ?",
        )
        .bind(provider_id)
        .bind(source.as_str())
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
                window_duration_mins,
                WindowObservation {
                    observed_at,
                    kind,
                    used_percent,
                    window_duration_mins,
                    resets_at,
                },
            );
        }
        Ok(observations)
    }

    pub(super) async fn insert_reset(
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

    /// Fills in the tokens each recorded restart carried, for the events whose hourly rows
    /// are all still stored.
    ///
    /// The window a restart closed runs from [`RESET_PREVIOUS_ANCHOR`] — or, for the first
    /// restart recorded of a window, the start its published expiry implies — to
    /// [`RESET_WINDOW_END`], and is held to its own length so that a restart nothing
    /// recorded between cannot credit one window with days of work. Hourly buckets are the
    /// finest resolution kept, so a bucket is credited to whichever window was running when
    /// it opened and the hour a restart falls in belongs to the window that starts there:
    /// no bucket is counted twice, and the total is approximate at the two boundaries only.
    ///
    /// The events outlive those rows, so a total is rebuilt on every write for as long as
    /// the hours behind it are complete and left alone once the oldest of them has been
    /// pruned — which is what keeps a restart from a year ago reporting the figure it was
    /// given at the time.
    pub(super) async fn refresh_reset_tokens(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
    ) -> Result<()> {
        let window_start = format!(
            "MAX(COALESCE({RESET_PREVIOUS_ANCHOR}, limit_resets.previous_resets_at - \
             limit_resets.window_duration_mins * 60), {RESET_WINDOW_END} - \
             limit_resets.window_duration_mins * 60)"
        );
        sqlx::query(&format!(
            "UPDATE limit_resets SET tokens_in_window = ( \
             SELECT SUM(total_tokens) FROM hourly_usage \
             WHERE hourly_usage.provider_instance_id = limit_resets.provider_instance_id \
             AND hour_start >= strftime('%Y-%m-%dT%H:00', {window_start}, 'unixepoch', \
               'localtime') \
             AND hour_start <= strftime('%Y-%m-%dT%H:00', limit_resets.previous_resets_at, \
               'unixepoch', 'localtime') \
             AND hour_start < strftime('%Y-%m-%dT%H:00', limit_resets.anchored_at, 'unixepoch', \
               'localtime')) \
             WHERE provider_instance_id = ? \
             AND date({window_start}, 'unixepoch', 'localtime') \
             >= date('now', 'localtime', '-{HOURLY_HISTORY_DAYS} days')"
        ))
        .bind(provider_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// The instant a rollout scan may start from, leaving enough overlap for a window
    /// that reset either side of the previous scan to still be paired with a reading.
    pub async fn reset_backfill_start(&self, provider: ProviderKind) -> Result<Option<i64>> {
        let last_completed: Option<String> =
            sqlx::query_scalar("SELECT last_completed_at FROM retention_state WHERE job_name = ?")
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
        Self::refresh_reset_tokens(&mut tx, provider_id).await?;
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
                    window_duration_mins: row
                        .try_get::<Option<i64>, _>("window_duration_mins")
                        .ok()??,
                    resets_at: row.try_get::<Option<i64>, _>("resets_at").ok()??,
                })
            })
            .collect())
    }

    /// Every restart recorded for a provider, newest first.
    ///
    /// Nothing prunes `limit_resets`, so this really is the whole history — which is why
    /// only the settings page asks for it and the dashboard takes
    /// [`load_recent_resets`](Self::load_recent_resets) instead.
    pub async fn load_reset_history(&self, provider: ProviderKind) -> Result<Vec<LimitResetEvent>> {
        let provider_id = self.provider_id(provider).await?;
        let rows = sqlx::query(&format!(
            "SELECT {RESET_COLUMNS} FROM limit_resets \
             WHERE provider_instance_id = ? ORDER BY anchored_at DESC"
        ))
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(reset_event).collect())
    }

    pub async fn load_recent_resets(&self, provider: ProviderKind) -> Result<Vec<LimitResetEvent>> {
        let provider_id = self.provider_id(provider).await?;
        let rows = sqlx::query(&format!(
            "SELECT {RESET_COLUMNS} FROM limit_resets \
             WHERE provider_instance_id = ? ORDER BY anchored_at DESC LIMIT ?"
        ))
        .bind(provider_id)
        .bind(RECENT_RESET_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(reset_event).collect())
    }

    /// What each quota window did across a date range, one point per local day.
    ///
    /// Two stores answer this between them and they do not overlap: readings younger than
    /// the retention cutoff are still in `limit_samples` at the granularity they arrived
    /// at, and everything older survives only as the daily rollups. A day is reduced to
    /// the highest share observed on it, so a window that filled and restarted the same
    /// day still reports how full it got.
    pub async fn load_quota_history(
        &self,
        provider: ProviderKind,
        start_date: &str,
        end_date: &str,
    ) -> Result<QuotaHistorySnapshot> {
        let start = jiff::civil::Date::from_str(start_date).context("invalid start date")?;
        let end = jiff::civil::Date::from_str(end_date).context("invalid end date")?;
        anyhow::ensure!(start <= end, "start date must not be after end date");
        let (start, end) = (start.to_string(), end.to_string());
        let provider_id = self.provider_id(provider).await?;

        // The rollups are day buckets already; only the raw samples have to be dated, and
        // they are dated locally so this chart shares the usage chart's calendar.
        let rows = sqlx::query(
            "SELECT day, window_kind, MAX(peak) AS peak, MAX(duration) AS duration FROM ( \
             SELECT date(observed_at, 'localtime') AS day, window_kind, \
             MAX(used_percent) AS peak, MAX(window_duration_mins) AS duration \
             FROM limit_samples WHERE provider_instance_id = ? AND used_percent IS NOT NULL \
             GROUP BY day, window_kind \
             UNION ALL \
             SELECT date(bucket_start) AS day, window_kind, \
             MAX(max_used_percent) AS peak, MAX(window_duration_mins) AS duration \
             FROM limit_rollups WHERE provider_instance_id = ? AND granularity = 'daily' \
             AND max_used_percent IS NOT NULL \
             GROUP BY day, window_kind \
             ) WHERE day BETWEEN ? AND ? GROUP BY day, window_kind ORDER BY day ASC",
        )
        .bind(provider_id)
        .bind(provider_id)
        .bind(&start)
        .bind(&end)
        .fetch_all(&self.pool)
        .await?;

        let mut windows: BTreeMap<LimitKind, (Option<i64>, Vec<QuotaHistoryPoint>)> =
            BTreeMap::new();
        for row in rows {
            let Some(kind) =
                row.try_get::<String, _>("window_kind").ok().as_deref().and_then(parse_kind)
            else {
                continue;
            };
            let window = windows.entry(kind).or_insert_with(|| (None, Vec::new()));
            window.0 = window.0.max(row.try_get::<Option<i64>, _>("duration").unwrap_or(None));
            window.1.push(QuotaHistoryPoint {
                date: row.get("day"),
                peak_used_percent: row.get::<f64, _>("peak"),
            });
        }

        let reset_rows = sqlx::query(&format!(
            "SELECT {RESET_COLUMNS} FROM limit_resets WHERE provider_instance_id = ? \
             AND date(anchored_at, 'unixepoch', 'localtime') BETWEEN ? AND ? \
             ORDER BY anchored_at ASC"
        ))
        .bind(provider_id)
        .bind(&start)
        .bind(&end)
        .fetch_all(&self.pool)
        .await?;

        Ok(QuotaHistorySnapshot {
            start_date: start,
            end_date: end,
            windows: windows
                .into_iter()
                .map(|(kind, (duration, points))| QuotaHistoryWindow {
                    kind,
                    label: kind.window_label(duration),
                    points,
                })
                .collect(),
            resets: reset_rows.into_iter().filter_map(reset_event).collect(),
        })
    }
}
