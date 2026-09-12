//! Usage that was measured on another computer, and the identity of this one.
//!
//! The shared folder carries rows and restarts between devices; nothing here reads a
//! provider, and an imported row is never counted as local.

use std::collections::BTreeMap;

use anyhow::Result;
use sqlx::Row;

use crate::domain::SharedResetEvent;
use crate::domain::{DeviceUsageRow, LimitResetEvent, ResetClassification};

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
}

/// One remote device's exported aggregates, ready to replace what is stored for it.
pub struct DeviceImport<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub parser_revision: &'a str,
    pub source_modified_at: i64,
    pub daily: &'a [DeviceUsageRow],
    pub hourly: &'a [DeviceUsageRow],
    pub resets: &'a [SharedResetEvent],
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
            "SELECT id, display_name, last_import_at, source_modified_at FROM devices \
             ORDER BY id = ? DESC, display_name ASC",
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

    /// The complete account-level set, including facts learned from another device. Each
    /// independent export therefore converges even if the original observer is offline.
    pub async fn load_reset_export(&self) -> Result<Vec<SharedResetEvent>> {
        let rows = sqlx::query(
            "SELECT provider_instances.provider, window_kind, window_duration_mins, anchored_at, \
             new_resets_at, previous_resets_at, used_percent_before, early_by_seconds, \
             classification, source, detected_at FROM limit_resets \
             JOIN provider_instances ON provider_instances.id = limit_resets.provider_instance_id \
             ORDER BY provider, window_duration_mins, new_resets_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(SharedResetEvent {
                    provider: row.try_get("provider").ok()?,
                    window_kind: parse_kind(&row.try_get::<String, _>("window_kind").ok()?)?,
                    window_duration_mins: row.try_get("window_duration_mins").ok()?,
                    anchored_at: row.try_get("anchored_at").ok()?,
                    new_resets_at: row.try_get("new_resets_at").ok()?,
                    previous_resets_at: row.try_get("previous_resets_at").ok()?,
                    used_percent_before: row.try_get("used_percent_before").ok()?,
                    early_by_seconds: row.try_get("early_by_seconds").ok()?,
                    classification: match row.try_get::<String, _>("classification").ok()?.as_str()
                    {
                        "unplanned" => ResetClassification::Unplanned,
                        _ => ResetClassification::Scheduled,
                    },
                    source: row.try_get("source").ok()?,
                    detected_at: row.try_get("detected_at").ok()?,
                })
            })
            .collect())
    }

    async fn load_export_rows(&self, table: &str, bucket: &str) -> Result<Vec<DeviceUsageRow>> {
        let rows = sqlx::query(&format!(
            "SELECT provider_instances.provider AS provider, {table}.{bucket} AS bucket, model, \
             service_tier, input_tokens, cache_read_tokens, output_tokens, reasoning_tokens, \
             total_tokens, estimated_cost_usd FROM {table} \
             JOIN provider_instances ON provider_instances.id = {table}.provider_instance_id \
             WHERE device = ? ORDER BY bucket ASC, model ASC"
        ))
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

    /// Replaces one remote device's rows with the set its exported file carries.
    ///
    /// Wholesale, because the file is that device's whole record: a day it no longer
    /// reports is a day it no longer has, and merging would keep a figure its own machine
    /// has already corrected. Rows naming a provider this build does not know are skipped
    /// — the other machine may be a version ahead — and the restart totals are rebuilt,
    /// since the hours a recorded window spans have just gained another machine's work.
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
            sqlx::query(&format!("DELETE FROM {table} WHERE device = ?"))
                .bind(device.id)
                .execute(&mut *tx)
                .await?;
        }

        let mut written = 0;
        for (table, bucket, rows) in [
            ("daily_usage", "usage_date", device.daily),
            ("hourly_usage", "hour_start", device.hourly),
        ] {
            for row in rows {
                let Some(&provider_id) = providers.get(&row.provider) else { continue };
                Self::insert_imported_row(
                    &mut tx,
                    provider_id,
                    device,
                    table,
                    bucket,
                    row,
                    imported_at,
                )
                .await?;
                written += 1;
            }
        }
        for reset in device.resets {
            let Some(&provider_id) = providers.get(&reset.provider) else { continue };
            if !matches!(reset.source.as_str(), "live" | "backfill") {
                continue;
            }
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
            };
            Self::insert_reset(&mut tx, provider_id, &event, &reset.source, &reset.detected_at)
                .await?;
        }
        for &provider_id in providers.values() {
            Self::refresh_reset_tokens(&mut tx, provider_id).await?;
        }
        tx.commit().await?;
        Ok(written)
    }

    async fn insert_imported_row(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        provider_id: i64,
        device: &DeviceImport<'_>,
        table: &str,
        bucket: &str,
        row: &DeviceUsageRow,
        imported_at: &str,
    ) -> Result<()> {
        sqlx::query(&format!(
            "INSERT INTO {table} \
             (provider_instance_id, device, {bucket}, model, service_tier, input_tokens, \
              cache_read_tokens, output_tokens, reasoning_tokens, total_tokens, \
              estimated_cost_usd, parser_revision, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
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
