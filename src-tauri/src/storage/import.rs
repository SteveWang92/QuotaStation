//! Usage that was measured on another computer, and the identity of this one.
//!
//! The shared folder carries rows and restarts between devices; nothing here reads a
//! provider, and an imported row is never counted as local.

use std::collections::BTreeMap;

use anyhow::Result;
use sqlx::{AssertSqlSafe, Row};

use crate::domain::SharedResetEvent;
use crate::domain::{DeviceUsageRow, LimitResetEvent};

use super::resets::{ResetObservation, parse_classification};
use super::{LOCAL_DEVICE, Storage, parse_kind};

/// A device as the database holds it. The local one is always present; the others arrive
/// with the first file read out of the shared folder.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub id: String,
    pub display_name: String,
    pub last_import_at: Option<String>,
    /// The modification time of the file this device's rows were read from, which is what
    /// decides whether the next refresh has to read it again.
    pub source_modified_at: Option<i64>,
    /// How many restart detections are attributed to this device.
    pub restart_count: i64,
}

/// One remote device's exported aggregates, ready to replace what is stored for it.
pub struct DeviceImport<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub parser_revision: &'a str,
    pub source_modified_at: i64,
    /// The daily and hourly rows, or `None` when they were refused. Refused rows leave the
    /// device's stored usage and its file's modification time alone, so the next refresh
    /// reads the file again and reports the refusal again.
    pub usage: Option<(&'a [DeviceUsageRow], &'a [DeviceUsageRow])>,
    /// Restarts are instants no time zone affects, so they are imported whatever happened
    /// to the rows.
    pub resets: &'a [SharedResetEvent],
    /// This machine's own shared identifier, which a detection relayed back to it carries.
    pub local_id: &'a str,
}

impl Storage {
    /// Names this machine in the device list, so a split reads "Workshop" rather than an
    /// identifier. Called whenever the name could have changed, which is startup and a
    /// settings save.
    pub async fn record_local_device(&self, display_name: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO devices (id, display_name) VALUES (?, ?) \
             ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name",
        )
        .bind(LOCAL_DEVICE)
        .bind(display_name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every device the totals are built from, this machine first.
    pub async fn load_devices(&self) -> Result<Vec<DeviceRecord>> {
        let rows = sqlx::query(
            "SELECT id, display_name, last_import_at, source_modified_at, ( \
               SELECT COUNT(*) FROM limit_reset_observations \
               WHERE limit_reset_observations.device = devices.id) AS restart_count \
             FROM devices ORDER BY id = ? DESC, display_name ASC",
        )
        .bind(LOCAL_DEVICE)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DeviceRecord {
                id: row.get("id"),
                display_name: row.get("display_name"),
                last_import_at: row.get("last_import_at"),
                source_modified_at: row.get("source_modified_at"),
                restart_count: row.get("restart_count"),
            })
            .collect())
    }

    /// This machine's own aggregates, as the shared folder carries them. Both resolutions
    /// come back whole: the exported file replaces its predecessor rather than adding to
    /// it, so a re-export repairs a file that was written wrong.
    pub async fn load_local_export(&self) -> Result<(Vec<DeviceUsageRow>, Vec<DeviceUsageRow>)> {
        Ok((
            self.load_export_rows("daily_usage", "usage_date").await?,
            self.load_export_rows("hourly_usage", "hour_start").await?,
        ))
    }

    /// Every attributed restart detection this machine knows, including ones learned from
    /// another device, so each independent export converges even while the device that saw
    /// a restart is offline. This machine's own are named by its shared identifier.
    ///
    /// A restart recorded before detections were attributed is left out: whichever device
    /// saw it, every device sharing at the time already has it, and a reader would credit
    /// it to this machine.
    pub async fn load_reset_export(
        &self,
        local_id: &str,
        local_name: &str,
    ) -> Result<Vec<SharedResetEvent>> {
        let rows = sqlx::query(
            "SELECT provider_instances.provider, window_kind, window_duration_mins, anchored_at, \
             new_resets_at, previous_resets_at, used_percent_before, early_by_seconds, \
             classification, source, detected_at, device, \
             COALESCE(devices.display_name, device_name) AS name, bracket_start, bracket_end \
             FROM limit_reset_observations \
             JOIN provider_instances \
               ON provider_instances.id = limit_reset_observations.provider_instance_id \
             LEFT JOIN devices ON devices.id = limit_reset_observations.device \
             WHERE device IS NOT NULL \
             ORDER BY provider, window_duration_mins, new_resets_at, device",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let device: String = row.try_get("device").ok()?;
                let (device_id, device_name) = if device == LOCAL_DEVICE {
                    (local_id.to_string(), Some(local_name.to_string()))
                } else {
                    (device, row.try_get("name").ok()?)
                };
                Some(SharedResetEvent {
                    provider: row.try_get("provider").ok()?,
                    window_kind: parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?,
                    window_duration_mins: row.try_get("window_duration_mins").ok()?,
                    anchored_at: row.try_get("anchored_at").ok()?,
                    new_resets_at: row.try_get("new_resets_at").ok()?,
                    previous_resets_at: row.try_get("previous_resets_at").ok()?,
                    used_percent_before: row.try_get("used_percent_before").ok()?,
                    early_by_seconds: row.try_get("early_by_seconds").ok()?,
                    classification: parse_classification(
                        &row.try_get::<String, _>("classification").ok()?,
                    ),
                    source: row.try_get("source").ok()?,
                    detected_at: row.try_get("detected_at").ok()?,
                    device_id: Some(device_id),
                    device_name,
                    bracket_start: row.try_get("bracket_start").ok()?,
                    bracket_end: row.try_get("bracket_end").ok()?,
                })
            })
            .collect())
    }

    async fn load_export_rows(
        &self,
        table: &'static str,
        bucket: &'static str,
    ) -> Result<Vec<DeviceUsageRow>> {
        // `table` and `bucket` are `&'static str` names, and every value is bound.
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT provider_instances.provider AS provider, {table}.{bucket} AS bucket, model, \
             service_tier, input_tokens, cache_read_tokens, output_tokens, reasoning_tokens, \
             total_tokens, estimated_cost_usd FROM {table} \
             JOIN provider_instances ON provider_instances.id = {table}.provider_instance_id \
             WHERE device = ? ORDER BY bucket ASC, model ASC"
        )))
        .bind(LOCAL_DEVICE)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DeviceUsageRow {
                provider: row.get("provider"),
                bucket: row.get("bucket"),
                model: row.get("model"),
                service_tier: row.get("service_tier"),
                input: row.get::<i64, _>("input_tokens") as u64,
                cache_read: row.get::<i64, _>("cache_read_tokens") as u64,
                output: row.get::<i64, _>("output_tokens") as u64,
                reasoning: row.get::<i64, _>("reasoning_tokens") as u64,
                total: row.get::<i64, _>("total_tokens") as u64,
                cost_usd: row.get("estimated_cost_usd"),
            })
            .collect())
    }

    /// Replaces one remote device's rows with the set its exported file carries, and merges
    /// the restart detections it relays.
    ///
    /// Rows are replaced wholesale, because the file is that device's whole record: a day
    /// it no longer reports is a day it no longer has, and merging would keep a figure its
    /// own machine has already corrected. Detections are merged instead, because the file
    /// carries other devices' as well as its own. Anything naming a provider this build
    /// does not know is skipped — the other machine may be a version ahead — and the
    /// restart totals are rebuilt, since the hours a recorded window spans have just gained
    /// another machine's work.
    pub async fn import_device(
        &self,
        device: &DeviceImport<'_>,
        imported_at: &str,
    ) -> Result<usize> {
        let providers: BTreeMap<String, i64> =
            sqlx::query("SELECT id, provider FROM provider_instances")
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| (row.get("provider"), row.get("id")))
                .collect();

        let mut tx = self.pool.begin().await?;
        let mut written = 0;
        if let Some((daily, hourly)) = device.usage {
            sqlx::query(
                "INSERT INTO devices (id, display_name, last_import_at, source_modified_at) \
                 VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET \
                   display_name = excluded.display_name, last_import_at = excluded.last_import_at, \
                   source_modified_at = excluded.source_modified_at",
            )
            .bind(device.id)
            .bind(device.display_name)
            .bind(imported_at)
            .bind(device.source_modified_at)
            .execute(&mut *tx)
            .await?;
            for table in ["daily_usage", "hourly_usage"] {
                // The table name comes from the literal list above, and every value is bound.
                sqlx::query(AssertSqlSafe(format!("DELETE FROM {table} WHERE device = ?")))
                    .bind(device.id)
                    .execute(&mut *tx)
                    .await?;
            }
            written =
                Self::insert_imported_rows(&mut tx, &providers, device, daily, hourly, imported_at)
                    .await?;
        } else {
            sqlx::query(
                "INSERT INTO devices (id, display_name) VALUES (?, ?) \
                 ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name",
            )
            .bind(device.id)
            .bind(device.display_name)
            .execute(&mut *tx)
            .await?;
        }
        for reset in device.resets {
            let Some(&provider_id) = providers.get(&reset.provider) else { continue };
            // A file an earlier build wrote attributes nothing, and everything in it is that
            // file's own device's.
            let (observer, observer_name) = match reset.device_id.as_deref() {
                None => (device.id, Some(device.display_name)),
                Some(id) if id == device.local_id => (LOCAL_DEVICE, None),
                Some(id) => (id, reset.device_name.as_deref()),
            };
            let event = LimitResetEvent {
                window_kind: reset.window_kind,
                window_label: reset.window_kind.window_label(Some(reset.window_duration_mins)),
                window_duration_mins: reset.window_duration_mins,
                anchored_at: reset.anchored_at,
                new_resets_at: reset.new_resets_at,
                previous_resets_at: reset.previous_resets_at,
                used_percent_before: reset.used_percent_before,
                tokens_in_window: None,
                early_by_seconds: reset.early_by_seconds,
                classification: reset.classification,
                anchor_spread_seconds: 0,
                detections: Vec::new(),
            };
            let observation = ResetObservation {
                device: observer,
                device_name: observer_name,
                event: &event,
                source: &reset.source,
                detected_at: &reset.detected_at,
                bracket: reset.bracket_start.zip(reset.bracket_end),
            };
            Self::merge_reset(&mut tx, provider_id, &observation).await?;
        }
        for &provider_id in providers.values() {
            Self::refresh_reset_tokens(&mut tx, provider_id).await?;
        }
        tx.commit().await?;
        Ok(written)
    }

    async fn insert_imported_rows(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        providers: &BTreeMap<String, i64>,
        device: &DeviceImport<'_>,
        daily: &[DeviceUsageRow],
        hourly: &[DeviceUsageRow],
        imported_at: &str,
    ) -> Result<usize> {
        let mut written = 0;
        for (table, bucket, rows) in
            [("daily_usage", "usage_date", daily), ("hourly_usage", "hour_start", hourly)]
        {
            for row in rows {
                let Some(&provider_id) = providers.get(&row.provider) else { continue };
                Self::insert_imported_row(tx, provider_id, device, table, bucket, row, imported_at)
                    .await?;
                written += 1;
            }
        }
        Ok(written)
    }

    async fn insert_imported_row(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        device: &DeviceImport<'_>,
        table: &'static str,
        bucket: &'static str,
        row: &DeviceUsageRow,
        imported_at: &str,
    ) -> Result<()> {
        // `table` and `bucket` are `&'static str` names, and every value is bound.
        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO {table} \
             (provider_instance_id, device, {bucket}, model, service_tier, input_tokens, \
              cache_read_tokens, output_tokens, reasoning_tokens, total_tokens, \
              estimated_cost_usd, parser_revision, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )))
        .bind(provider_id)
        .bind(device.id)
        .bind(&row.bucket)
        .bind(&row.model)
        .bind(&row.service_tier)
        .bind(row.input as i64)
        .bind(row.cache_read as i64)
        .bind(row.output as i64)
        .bind(row.reasoning as i64)
        .bind(row.total as i64)
        .bind(row.cost_usd)
        // The revision that parsed these rows is the exporting machine's, not this one's.
        .bind(device.parser_revision)
        .bind(imported_at)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}
