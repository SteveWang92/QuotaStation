//! What the quota readings leave behind: each reading as it is taken, the window it
//! belongs to, and the restart the tracker infers from the pair.
//!
//! `crate::resets` decides that a window restarted; this stores the decision and reads it
//! back. Each device's detection of a restart is an observation, and the restart itself is
//! the event those observations merge into — see [`Storage::merge_reset`].

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use anyhow::{Context, Result};
use sqlx::{AssertSqlSafe, Row};

use crate::domain::{
    HOURLY_HISTORY_DAYS, LimitKind, LimitResetEvent, LiveSnapshot, QuotaHistoryPoint,
    QuotaHistorySnapshot, QuotaHistoryWindow, ResetClassification, ResetDetection, WindowSource,
};
use crate::providers::ProviderKind;
use crate::resets::{ResetTracker, WindowObservation, detect, grouping_tolerance_seconds};

use super::{LOCAL_DEVICE, Storage, epoch_seconds, parse_kind};

/// Which of a restart's observations speaks for it: the one whose two readings were closest
/// together, because the restart happened between them; then a live read before a rollout
/// log; then the device, so the choice never depends on the order observations arrived in.
/// A detection recorded before brackets were kept has none and comes last.
const REPRESENTATIVE_ORDER: &str = "bracket_end - bracket_start IS NULL, \
    bracket_end - bracket_start, source <> 'live', device, anchored_at";

/// One device's detection of a restart, as [`Storage::merge_reset`] files it.
pub(super) struct ResetObservation<'a> {
    /// [`LOCAL_DEVICE`] or another device's shared identifier.
    pub device: &'a str,
    /// The name a device reported itself by, kept for a device whose own file this machine
    /// has not read. `None` for this machine.
    pub device_name: Option<&'a str>,
    pub event: &'a LimitResetEvent,
    pub source: &'a str,
    pub detected_at: &'a str,
    pub bracket: Option<(i64, i64)>,
}

/// The rollout scan skips files older than its previous run, with this much overlap so a
/// window that reset across the boundary still has an earlier reading to compare against.
const BACKFILL_OVERLAP_HOURS: i64 = 48;

/// How many restarts the surfaces are given. They annotate the window running now and
/// list the ones before it, neither of which needs the whole history.
const RECENT_RESET_LIMIT: i64 = 8;

/// The columns every query over `limit_resets` selects. The token total among them is
/// stored rather than summed on the way out, so it outlives the hourly rows it was built
/// from — see [`Storage::refresh_reset_tokens`].
const RESET_COLUMNS: &str = "id, window_kind, window_duration_mins, anchored_at, new_resets_at, \
    previous_resets_at, used_percent_before, tokens_in_window, early_by_seconds, classification, \
    anchor_spread_seconds";

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

pub(super) fn parse_classification(value: &str) -> ResetClassification {
    match value {
        "unplanned" => ResetClassification::Unplanned,
        _ => ResetClassification::Scheduled,
    }
}

/// One recorded restart and its row id, from a row carrying [`RESET_COLUMNS`]. The recent
/// list and the range query select the same columns, so they read them the same way; the
/// detections are attached by [`Storage::with_detections`].
fn reset_event(row: sqlx::sqlite::SqliteRow) -> Option<(i64, LimitResetEvent)> {
    let kind = parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?;
    let window_duration_mins: i64 = row.try_get("window_duration_mins").ok()?;
    let classification = parse_classification(&row.try_get::<String, _>("classification").ok()?);
    let event = LimitResetEvent {
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
        anchor_spread_seconds: row.try_get("anchor_spread_seconds").ok()?,
        detections: Vec::new(),
    };
    Some((row.try_get("id").ok()?, event))
}

/// The version of the replay: how observations are read out of the logs and how a restart is
/// detected among them. It moves whenever either changes, so a watermark an older replay
/// reached is not trusted and the logs are replayed from the start.
const RESET_BACKFILL_VERSION: i64 = 1;

/// Each provider replays its own logs, so the scan watermarks cannot share a row.
fn backfill_job_name(provider: ProviderKind) -> String {
    format!("{}_reset_backfill", provider.key())
}

impl Storage {
    /// Stores a live reading, and reports whether it recorded a restart nothing had recorded
    /// before — which is news the other devices should hear without waiting for the next
    /// history refresh.
    pub async fn save_live(
        &self,
        provider: ProviderKind,
        live: &LiveSnapshot,
        observed_at: &str,
    ) -> Result<bool> {
        let provider_id = self.provider_id(provider).await?;
        let mut recorded = false;
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
                    let observation = ResetObservation {
                        device: LOCAL_DEVICE,
                        device_name: None,
                        event: &event,
                        source: "live",
                        detected_at: observed_at,
                        bracket: Some((earlier.observed_at, current.observed_at)),
                    };
                    recorded |= Self::merge_reset(&mut tx, provider_id, &observation).await?;
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
        Ok(recorded)
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

    /// Files one device's detection of a restart, and reports whether it began a restart
    /// nothing had recorded before. Live detection, the rollout backfill and the shared
    /// folder all write restarts through here and nowhere else.
    ///
    /// The detection joins the recorded restart of its window whose anchor is nearest within
    /// [`grouping_tolerance_seconds`], or starts one of its own. Real restarts of one window
    /// are hours apart, so a chain of detections each close to the next but not to the
    /// first cannot arise and is not re-clustered. The restart's fields are then rewritten
    /// from its [`REPRESENTATIVE_ORDER`] observation; the others are kept beside it rather
    /// than voted on. A detection a device already filed is left as it was, which is what
    /// makes reading the same file twice harmless.
    pub(super) async fn merge_reset(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        observation: &ResetObservation<'_>,
    ) -> Result<bool> {
        let event = observation.event;
        let filed: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM limit_reset_observations WHERE provider_instance_id = ? \
             AND device = ? AND window_duration_mins = ? AND new_resets_at = ?",
        )
        .bind(provider_id)
        .bind(observation.device)
        .bind(event.window_duration_mins)
        .bind(event.new_resets_at)
        .fetch_optional(&mut **tx)
        .await?;
        if filed.is_some() {
            return Ok(false);
        }

        let joined: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM limit_resets WHERE provider_instance_id = ? \
             AND window_duration_mins = ? AND ABS(anchored_at - ?) <= ? \
             ORDER BY ABS(anchored_at - ?), id LIMIT 1",
        )
        .bind(provider_id)
        .bind(event.window_duration_mins)
        .bind(event.anchored_at)
        .bind(grouping_tolerance_seconds(event.window_duration_mins))
        .bind(event.anchored_at)
        .fetch_optional(&mut **tx)
        .await?;
        let (reset_id, created) = match joined {
            Some(reset_id) => (reset_id, false),
            None => {
                let reset_id = sqlx::query_scalar(
                    "INSERT INTO limit_resets \
                     (provider_instance_id, window_kind, window_duration_mins, anchored_at, \
                      new_resets_at, previous_resets_at, used_percent_before, early_by_seconds, \
                      classification, source, detected_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
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
                .bind(observation.source)
                .bind(observation.detected_at)
                .fetch_one(&mut **tx)
                .await?;
                (reset_id, true)
            }
        };

        sqlx::query(
            "INSERT INTO limit_reset_observations \
             (reset_id, provider_instance_id, device, device_name, window_kind, \
              window_duration_mins, anchored_at, new_resets_at, previous_resets_at, \
              used_percent_before, early_by_seconds, classification, source, detected_at, \
              bracket_start, bracket_end) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(reset_id)
        .bind(provider_id)
        .bind(observation.device)
        .bind(observation.device_name)
        .bind(kind_column(event.window_kind))
        .bind(event.window_duration_mins)
        .bind(event.anchored_at)
        .bind(event.new_resets_at)
        .bind(event.previous_resets_at)
        .bind(event.used_percent_before)
        .bind(event.early_by_seconds)
        .bind(event.classification.as_str())
        .bind(observation.source)
        .bind(observation.detected_at)
        .bind(observation.bracket.map(|(start, _)| start))
        .bind(observation.bracket.map(|(_, end)| end))
        .execute(&mut **tx)
        .await?;
        // A restart recorded before detections were attributed stands in for a device
        // nobody knew; an attributed detection of the same restart is that device.
        sqlx::query("DELETE FROM limit_reset_observations WHERE reset_id = ? AND device IS NULL")
            .bind(reset_id)
            .execute(&mut **tx)
            .await?;
        // `REPRESENTATIVE_ORDER` is a constant, and the row id is bound.
        sqlx::query(AssertSqlSafe(format!(
            "UPDATE limit_resets SET (window_kind, anchored_at, new_resets_at, previous_resets_at, \
               used_percent_before, early_by_seconds, classification, source, detected_at) = ( \
               SELECT window_kind, anchored_at, new_resets_at, previous_resets_at, \
               used_percent_before, early_by_seconds, classification, source, detected_at \
               FROM limit_reset_observations WHERE reset_id = limit_resets.id \
               ORDER BY {REPRESENTATIVE_ORDER} LIMIT 1), \
             anchor_spread_seconds = ( \
               SELECT MAX(anchored_at) - MIN(anchored_at) FROM limit_reset_observations \
               WHERE reset_id = limit_resets.id) \
             WHERE id = ?"
        )))
        .bind(reset_id)
        .execute(&mut **tx)
        .await?;
        Ok(created)
    }

    /// Attaches every device's detection to the restarts read back, the representative's
    /// first, and drops the row ids they were matched by.
    async fn with_detections(
        &self,
        rows: Vec<sqlx::sqlite::SqliteRow>,
    ) -> Result<Vec<LimitResetEvent>> {
        let events: Vec<(i64, LimitResetEvent)> =
            rows.into_iter().filter_map(reset_event).collect();
        if events.is_empty() {
            return Ok(Vec::new());
        }
        // The ids are integers read back from the database and `REPRESENTATIVE_ORDER` is a
        // constant, so only digits and fixed SQL reach the statement.
        let ids = events.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>().join(",");
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT reset_id, device, COALESCE(devices.display_name, device_name) AS name, \
             source, anchored_at, classification FROM limit_reset_observations \
             LEFT JOIN devices ON devices.id = limit_reset_observations.device \
             WHERE reset_id IN ({ids}) ORDER BY {REPRESENTATIVE_ORDER}"
        )))
        .fetch_all(&self.pool)
        .await?;
        let mut detections: BTreeMap<i64, Vec<ResetDetection>> = BTreeMap::new();
        for row in rows {
            let device: Option<String> = row.try_get("device")?;
            detections.entry(row.try_get("reset_id")?).or_default().push(ResetDetection {
                local: device.as_deref() == Some(LOCAL_DEVICE),
                device_name: device.and(row.try_get("name")?),
                source: row.try_get("source")?,
                anchored_at: row.try_get("anchored_at")?,
                classification: parse_classification(&row.try_get::<String, _>("classification")?),
            });
        }
        Ok(events
            .into_iter()
            .map(|(id, mut event)| {
                event.detections = detections.remove(&id).unwrap_or_default();
                event
            })
            .collect())
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
        // Hour keys are local to the application zone, so the instants bounding each window
        // are read out and keyed here rather than by SQLite, which knows only the Windows zone.
        let window_start = format!(
            "MAX(COALESCE({RESET_PREVIOUS_ANCHOR}, limit_resets.previous_resets_at - \
             limit_resets.window_duration_mins * 60), {RESET_WINDOW_END} - \
             limit_resets.window_duration_mins * 60)"
        );
        let oldest_hour = crate::clock::day_start(
            crate::clock::today().saturating_sub(jiff::Span::new().days(HOURLY_HISTORY_DAYS)),
        )?;
        // `window_start` is built from constants alone, and every value is bound.
        let windows = sqlx::query(AssertSqlSafe(format!(
            "SELECT id, {window_start} AS window_start, previous_resets_at, anchored_at \
             FROM limit_resets WHERE provider_instance_id = ? AND {window_start} >= ?"
        )))
        .bind(provider_id)
        .bind(oldest_hour)
        .fetch_all(&mut **tx)
        .await?;
        for window in windows {
            let (Some(first), Some(expiry), Some(restart)) = (
                crate::clock::hour_key(window.try_get("window_start")?),
                crate::clock::hour_key(window.try_get("previous_resets_at")?),
                crate::clock::hour_key(window.try_get("anchored_at")?),
            ) else {
                continue;
            };
            sqlx::query(
                "UPDATE limit_resets SET tokens_in_window = ( \
                 SELECT SUM(total_tokens) FROM hourly_usage \
                 WHERE provider_instance_id = ? AND hour_start >= ? AND hour_start <= ? \
                 AND hour_start < ?) WHERE id = ?",
            )
            .bind(provider_id)
            .bind(first)
            .bind(expiry)
            .bind(restart)
            .bind(window.try_get::<i64, _>("id")?)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// The instant a rollout scan may start from, leaving enough overlap for a window
    /// that reset either side of the previous scan to still be paired with a reading.
    /// `None`, a scan from the start, when no scan has finished at this replay version.
    pub async fn reset_backfill_start(&self, provider: ProviderKind) -> Result<Option<i64>> {
        let watermark: Option<(Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT last_completed_at, reader_version FROM retention_state WHERE job_name = ?",
        )
        .bind(backfill_job_name(provider))
        .fetch_optional(&self.pool)
        .await?;
        Ok(watermark
            .filter(|(_, version)| version.is_some_and(|version| version >= RESET_BACKFILL_VERSION))
            .and_then(|(completed, _)| completed)
            .as_deref()
            .and_then(epoch_seconds)
            .map(|completed| completed - BACKFILL_OVERLAP_HOURS * 3_600))
    }

    /// Replays the readings a provider's client kept for itself — Codex's rollout logs,
    /// Claude's status-line record — so restarts that happened while QuotaStation was closed
    /// are still recorded. A restart another device already shared joins that restart as
    /// this machine's own detection of it; storing a detection twice is prevented by the
    /// table, not by the caller.
    pub async fn backfill_resets(
        &self,
        provider: ProviderKind,
        observations: &[WindowObservation],
        scanned_at: &str,
    ) -> Result<usize> {
        let provider_id = self.provider_id(provider).await?;
        let mut merged = observations.to_vec();
        // Codex's rollout logs lack the readings this machine took from the app server, so
        // the stored samples fill them in. Claude's record already holds every reading the
        // samples were copied from, and the samples carry no source that would keep a
        // log-derived window apart from a published one.
        if provider == ProviderKind::Codex {
            merged.extend(self.load_sample_observations(provider_id).await?);
        }
        merged.sort_by_key(|observation| observation.observed_at);

        let mut tracker = ResetTracker::default();
        let mut detections = Vec::new();
        for observation in merged {
            if let Some(detection) = tracker.push(observation) {
                detections.push(detection);
            }
        }

        let mut tx = self.pool.begin().await?;
        for detection in &detections {
            let observation = ResetObservation {
                device: LOCAL_DEVICE,
                device_name: None,
                event: &detection.event,
                source: "backfill",
                detected_at: scanned_at,
                bracket: Some(detection.bracket),
            };
            Self::merge_reset(&mut tx, provider_id, &observation).await?;
        }
        Self::refresh_reset_tokens(&mut tx, provider_id).await?;
        sqlx::query(
            "INSERT INTO retention_state \
             (job_name, last_completed_at, last_status, last_error, reader_version) \
             VALUES (?, ?, 'succeeded', NULL, ?) ON CONFLICT(job_name) DO UPDATE SET \
             last_completed_at=excluded.last_completed_at, last_status=excluded.last_status, \
             last_error=NULL, reader_version=excluded.reader_version",
        )
        .bind(backfill_job_name(provider))
        .bind(scanned_at)
        .bind(RESET_BACKFILL_VERSION)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(detections.len())
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
        // `RESET_COLUMNS` is a constant, and every value is bound.
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {RESET_COLUMNS} FROM limit_resets \
             WHERE provider_instance_id = ? ORDER BY anchored_at DESC"
        )))
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        self.with_detections(rows).await
    }

    pub async fn load_recent_resets(&self, provider: ProviderKind) -> Result<Vec<LimitResetEvent>> {
        let provider_id = self.provider_id(provider).await?;
        // `RESET_COLUMNS` is a constant, and every value is bound.
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {RESET_COLUMNS} FROM limit_resets \
             WHERE provider_instance_id = ? ORDER BY anchored_at DESC LIMIT ?"
        )))
        .bind(provider_id)
        .bind(RECENT_RESET_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        self.with_detections(rows).await
    }

    /// What each quota window did across a date range, one point per local day.
    ///
    /// Two stores answer this between them and they do not overlap: `limit_samples` keeps
    /// every reading at the granularity it arrived at, and `limit_rollups` holds daily
    /// summaries only for days whose readings are no longer stored. A day is reduced to
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
        // The instants the local range spans in the application zone, which a stored instant
        // is compared against instead of being dated by SQLite in the Windows zone.
        let (from, until) =
            (crate::clock::day_start(start)?, crate::clock::day_start(end.tomorrow()?)?);
        let (start, end) = (start.to_string(), end.to_string());
        let provider_id = self.provider_id(provider).await?;

        // Each day's peak per window, from whichever of the two stores holds that day.
        let mut days: BTreeMap<(LimitKind, String), (f64, Option<i64>)> = BTreeMap::new();
        let mut record = |kind: LimitKind, day: String, peak: f64, duration: Option<i64>| {
            let entry = days.entry((kind, day)).or_insert((peak, duration));
            entry.0 = entry.0.max(peak);
            entry.1 = entry.1.max(duration);
        };

        // The rollups are day buckets already, keyed when they were rolled up; only the raw
        // samples have to be dated, and they are dated in the application zone so this chart
        // shares the usage chart's calendar.
        // Readings are kept for good, so the range is also compared as text to let the index
        // narrow it. Every reading is stored as a UTC RFC 3339 string, which sorts by time
        // to the second; the text bounds sit a second outside the range and the exact
        // comparison decides the edges.
        let text_bound =
            |epoch: i64| jiff::Timestamp::from_second(epoch).map(|instant| instant.to_string());
        let samples = sqlx::query(
            "SELECT unixepoch(observed_at) AS observed, window_kind, used_percent, \
             window_duration_mins FROM limit_samples WHERE provider_instance_id = ? \
             AND observed_at > ? AND observed_at < ? \
             AND used_percent IS NOT NULL AND unixepoch(observed_at) >= ? \
             AND unixepoch(observed_at) < ?",
        )
        .bind(provider_id)
        .bind(text_bound(from - 1)?)
        .bind(text_bound(until + 1)?)
        .bind(from)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;
        for sample in samples {
            let (Some(kind), Some(day)) = (
                parse_kind(&sample.try_get::<String, _>("window_kind")?),
                crate::clock::day_key(sample.try_get("observed")?),
            ) else {
                continue;
            };
            record(
                kind,
                day,
                sample.try_get("used_percent")?,
                sample.try_get("window_duration_mins")?,
            );
        }
        let rollups = sqlx::query(
            "SELECT date(bucket_start) AS day, window_kind, MAX(max_used_percent) AS peak, \
             MAX(window_duration_mins) AS duration FROM limit_rollups \
             WHERE provider_instance_id = ? AND granularity = 'daily' \
             AND max_used_percent IS NOT NULL AND date(bucket_start) BETWEEN ? AND ? \
             GROUP BY day, window_kind",
        )
        .bind(provider_id)
        .bind(&start)
        .bind(&end)
        .fetch_all(&self.pool)
        .await?;
        for rollup in rollups {
            let Some(kind) = parse_kind(&rollup.try_get::<String, _>("window_kind")?) else {
                continue;
            };
            record(
                kind,
                rollup.try_get("day")?,
                rollup.try_get("peak")?,
                rollup.try_get("duration")?,
            );
        }

        let mut windows: BTreeMap<LimitKind, (Option<i64>, Vec<QuotaHistoryPoint>)> =
            BTreeMap::new();
        for ((kind, date), (peak_used_percent, duration)) in days {
            let window = windows.entry(kind).or_insert_with(|| (None, Vec::new()));
            window.0 = window.0.max(duration);
            window.1.push(QuotaHistoryPoint { date, peak_used_percent });
        }

        // `RESET_COLUMNS` is a constant, and every value is bound.
        let reset_rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {RESET_COLUMNS} FROM limit_resets WHERE provider_instance_id = ? \
             AND anchored_at >= ? AND anchored_at < ? ORDER BY anchored_at ASC"
        )))
        .bind(provider_id)
        .bind(from)
        .bind(until)
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
            resets: self.with_detections(reset_rows).await?,
        })
    }
}
