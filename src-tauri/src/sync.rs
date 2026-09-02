//! Usage aggregates shared between this user's own machines.
//!
//! Quota percentages are account-wide and come from the provider's server, but token
//! totals are parsed from one machine's session logs — so on a second machine every figure
//! built from them under-counts by whatever the other one did. Each machine writes its own
//! normalized aggregates into a folder the user already keeps in sync and reads every other
//! machine's out of it. Both machines run the same code in both directions: there is no
//! server, no primary machine, and no merge conflict, because a machine only ever writes
//! its own file.
//!
//! What travels is the aggregate vocabulary and nothing else — a device name, the zone the
//! buckets were keyed in, and per-hour and per-day per-model token counts and costs. No
//! project names, no paths, no session identifiers, no prompts, no account identity. That
//! is the whole reason aggregates are exported rather than the session logs themselves,
//! which would work with no code at all and would put prompts in a sync folder.

use std::{path::Path, str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    domain::{CCUSAGE_REVISION, DeviceUsageRow, SharedFolderDiagnostics, SharedResetEvent},
    resets::{MAX_WINDOW_DURATION_MINS, UNPLANNED_THRESHOLD_SECONDS},
    sanitize::sanitize_error,
    storage::DeviceImport,
};

/// The shape a reader expects. A file written by a newer build is left alone rather than
/// read as though this one understood it.
const FORMAT_VERSION: u32 = 1;

const FILE_PREFIX: &str = "usage-";
const FILE_SUFFIX: &str = ".json";
const MAX_DEVICE_ID_LEN: usize = 32;

/// One machine's whole record, as the shared folder carries it.
///
/// There is deliberately no export timestamp: the file's own modification time is when it
/// was written, and a field that changed on every export would make every refresh rewrite
/// a file whose numbers had not moved — and every sync client upload it again.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    format_version: u32,
    device_id: String,
    device_name: String,
    /// The time zone the buckets below are keyed in. Local hour and day keys from two
    /// zones describe different spans of time and cannot be added together, so an import
    /// from a machine on another zone is refused rather than shifted.
    timezone: String,
    parser_revision: String,
    daily: Vec<DeviceUsageRow>,
    hourly: Vec<DeviceUsageRow>,
    /// Optional so an upgraded machine still imports a pre-reset-sharing file.
    #[serde(default)]
    resets: Vec<SharedResetEvent>,
}

fn file_name(device_id: &str) -> String {
    format!("{FILE_PREFIX}{device_id}{FILE_SUFFIX}")
}

fn valid_device_id(device_id: &str) -> bool {
    !device_id.is_empty()
        && device_id.len() <= MAX_DEVICE_ID_LEN
        && device_id.chars().all(|character| character.is_ascii_hexdigit())
}

fn system_timezone() -> String {
    jiff::tz::TimeZone::system().iana_name().unwrap_or("UTC").to_string()
}

/// Publishes this machine's aggregates and reads in whatever the others have published.
///
/// Runs after every refresh rather than behind a button: a machine used for a fortnight and
/// then opened once must not need anyone to remember anything. A failure at either end is
/// reported and nothing else — a shared folder that is unreachable is not a reason to fail
/// the refresh that produced the numbers.
pub async fn run(state: &Arc<AppState>) -> SharedFolderDiagnostics {
    let settings = state.settings();
    let (Some(folder), Some(device_id)) = (settings.shared_usage_folder, settings.device_id) else {
        return SharedFolderDiagnostics::default();
    };
    let folder = Path::new(&folder);
    let device_name = settings.device_name.unwrap_or_else(crate::settings::default_device_name);

    let mut failures = Vec::new();
    if let Err(error) = export(state, folder, &device_id, &device_name).await {
        failures.push(sanitize_error(&error.to_string(), "Export failed"));
    }
    failures.extend(import_others(state, folder, &device_id).await);

    let completed_at = jiff::Timestamp::now().to_string();
    // The folder itself is never named here: the whole point of the shared folder is that
    // it lives wherever the user's sync client keeps it.
    crate::log::write(if failures.is_empty() {
        "shared usage folder exchanged".to_string()
    } else {
        format!("shared usage folder: {}", failures.join(" · "))
    });
    if failures.is_empty() {
        SharedFolderDiagnostics {
            status: "succeeded".to_string(),
            last_completed_at: Some(completed_at),
            error: None,
        }
    } else {
        SharedFolderDiagnostics {
            status: "failed".to_string(),
            last_completed_at: Some(completed_at),
            error: Some(failures.join(" · ")),
        }
    }
}

/// Writes this machine's whole hourly and daily set, as one file that replaces its
/// predecessor. A full replacement rather than an append is what lets a re-export repair a
/// file that was written wrong.
async fn export(
    state: &Arc<AppState>,
    folder: &Path,
    device_id: &str,
    device_name: &str,
) -> Result<()> {
    // The folder is not created: a chosen folder that is not there is a folder no sync
    // client is keeping in step, and silently making a local one would leave this machine
    // exporting into nowhere for as long as it took someone to notice.
    anyhow::ensure!(folder.is_dir(), "The shared usage folder was not found.");
    let (daily, hourly) = state.storage.load_local_export().await?;
    let resets = state.storage.load_reset_export().await?;
    let content = serde_json::to_vec(&ExportFile {
        format_version: FORMAT_VERSION,
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        timezone: system_timezone(),
        parser_revision: CCUSAGE_REVISION.to_string(),
        daily,
        hourly,
        resets,
    })?;

    let path = folder.join(file_name(device_id));
    if std::fs::read(&path).is_ok_and(|published| published == content) {
        return Ok(());
    }
    // Written beside the file and renamed onto it, so a sync client watching the folder
    // never picks up half a document.
    let staging = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&staging, &content).context("write the exported aggregates")?;
    std::fs::rename(&staging, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })?;
    Ok(())
}

/// Reads every other machine's file that has moved since it was last read.
///
/// A file that cannot be read is skipped with a diagnostic and nothing more: a sync client
/// halfway through replacing one is an ordinary event, and the next refresh finds it whole.
async fn import_others(state: &Arc<AppState>, folder: &Path, device_id: &str) -> Vec<String> {
    let known = match state.storage.load_devices().await {
        Ok(devices) => devices,
        Err(error) => return vec![sanitize_error(&error.to_string(), "Import failed")],
    };
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) => return vec![sanitize_error(&error.to_string(), "Import failed")],
    };

    let mut failures = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else { continue };
        let Some(file_device) =
            name.strip_prefix(FILE_PREFIX).and_then(|rest| rest.strip_suffix(FILE_SUFFIX))
        else {
            continue;
        };
        // A device identifier is hexadecimal, so anything else between the prefix and the
        // suffix belongs to the sync client rather than to a machine: Proton Drive and
        // Dropbox both name their conflict copies by decorating the original file name.
        // Such a copy is a duplicate of a file already read, and reporting it as an
        // unreadable device would leave the shared-folder status failing until someone
        // deleted a file the sync client is entitled to create.
        if !valid_device_id(file_device) {
            continue;
        }
        if file_device == device_id {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| jiff::Timestamp::try_from(modified).ok())
            .map(|modified| modified.as_second());
        let Some(modified) = modified else { continue };
        if known
            .iter()
            .any(|device| device.id == file_device && device.source_modified_at == Some(modified))
        {
            continue;
        }
        if let Err(error) = import_one(state, &entry.path(), file_device, modified).await {
            failures.push(format!("{name}: {}", sanitize_error(&error.to_string(), "unreadable")));
        }
    }
    failures
}

/// The length of a local day key (`2026-08-24`) and of a local hour key (`2026-08-24T09:00`).
const DAY_KEY_LEN: usize = 10;
const HOUR_KEY_LEN: usize = 16;

/// Refuses rows that cannot have been produced by this build before SQLite sees them.
fn check_rows(rows: &[DeviceUsageRow], hourly: bool) -> Result<()> {
    for row in rows {
        let date = row.bucket.get(..DAY_KEY_LEN).unwrap_or_default();
        anyhow::ensure!(
            jiff::civil::Date::from_str(date).is_ok_and(|parsed| parsed.to_string() == date)
                && if hourly {
                    row.bucket.len() == HOUR_KEY_LEN
                        && row.bucket.as_bytes().get(10) == Some(&b'T')
                        && row.bucket.as_bytes().get(13) == Some(&b':')
                        && row.bucket.get(14..) == Some("00")
                        && row
                            .bucket
                            .get(11..13)
                            .and_then(|hour| hour.parse::<u8>().ok())
                            .is_some_and(|hour| hour < 24)
                } else {
                    row.bucket.len() == DAY_KEY_LEN
                },
            "carries an unreadable bucket key"
        );
        anyhow::ensure!(
            [row.input, row.cache_read, row.output, row.reasoning, row.total]
                .into_iter()
                .all(|tokens| i64::try_from(tokens).is_ok()),
            "carries a token total outside the supported range"
        );
        anyhow::ensure!(
            row.cost_usd.is_none_or(|cost| cost.is_finite() && cost >= 0.0),
            "carries an invalid cost"
        );
    }
    Ok(())
}

fn check_resets(resets: &[SharedResetEvent]) -> Result<()> {
    for reset in resets {
        anyhow::ensure!(
            matches!(reset.source.as_str(), "live" | "backfill"),
            "carries an invalid reset source"
        );
        anyhow::ensure!(
            (1..=MAX_WINDOW_DURATION_MINS).contains(&reset.window_duration_mins)
                && (0.0..=100.0).contains(&reset.used_percent_before),
            "carries invalid reset values"
        );
        let duration_seconds = reset.window_duration_mins.checked_mul(60);
        anyhow::ensure!(
            duration_seconds.and_then(|duration| reset.anchored_at.checked_add(duration))
                == Some(reset.new_resets_at)
                && reset.previous_resets_at.checked_sub(reset.anchored_at)
                    == Some(reset.early_by_seconds),
            "carries an inconsistent reset window"
        );
        let expected_classification = if reset.early_by_seconds > UNPLANNED_THRESHOLD_SECONDS {
            crate::domain::ResetClassification::Unplanned
        } else {
            crate::domain::ResetClassification::Scheduled
        };
        anyhow::ensure!(
            reset.classification == expected_classification,
            "carries an inconsistent reset classification"
        );
        anyhow::ensure!(
            jiff::Timestamp::from_str(&reset.detected_at).is_ok(),
            "carries an unreadable reset timestamp"
        );
    }
    Ok(())
}

async fn import_one(
    state: &Arc<AppState>,
    path: &Path,
    file_device: &str,
    modified: i64,
) -> Result<()> {
    let published: ExportFile = serde_json::from_slice(&std::fs::read(path)?)?;
    anyhow::ensure!(
        published.format_version == FORMAT_VERSION,
        "written by a different version of QuotaStation"
    );
    anyhow::ensure!(published.device_id == file_device, "names a different device inside");
    anyhow::ensure!(valid_device_id(&published.device_id), "names an invalid device");
    let timezone = system_timezone();
    anyhow::ensure!(
        published.timezone == timezone,
        "aggregated in {} rather than {timezone}",
        published.timezone
    );
    // The imported values are bound directly into SQLite and later read as signed integers,
    // dates and reset facts, so the external document has to fit those exact representations.
    check_rows(&published.daily, false)?;
    check_rows(&published.hourly, true)?;
    check_resets(&published.resets)?;

    state
        .storage
        .import_device(
            &DeviceImport {
                id: &published.device_id,
                display_name: &published.device_name,
                parser_revision: &published.parser_revision,
                source_modified_at: modified,
                daily: &published.daily,
                hourly: &published.hourly,
                resets: &published.resets,
            },
            &jiff::Timestamp::now().to_string(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(bucket: &str) -> DeviceUsageRow {
        DeviceUsageRow {
            provider: "codex".into(),
            bucket: bucket.into(),
            model: "gpt-5".into(),
            service_tier: "default".into(),
            input: 1,
            cache_read: 2,
            output: 3,
            reasoning: 4,
            total: 10,
            cost_usd: Some(0.25),
        }
    }

    #[test]
    fn shared_rows_require_real_canonical_bucket_keys() {
        assert!(check_rows(&[row("2026-08-29")], false).is_ok());
        assert!(check_rows(&[row("2026-08-29T23:00")], true).is_ok());
        assert!(check_rows(&[row("2026-02-29")], false).is_err());
        assert!(check_rows(&[row("2026-08-29T24:00")], true).is_err());
        assert!(check_rows(&[row("2026-08-29T23:30")], true).is_err());
    }

    #[test]
    fn shared_rows_fit_the_database_number_types() {
        let mut invalid_tokens = row("2026-08-29");
        invalid_tokens.total = i64::MAX as u64 + 1;
        assert!(check_rows(&[invalid_tokens], false).is_err());

        let mut invalid_cost = row("2026-08-29");
        invalid_cost.cost_usd = Some(-0.01);
        assert!(check_rows(&[invalid_cost], false).is_err());
    }

    #[test]
    fn device_ids_cannot_be_empty_or_unbounded() {
        assert!(valid_device_id("18dc7f42a1"));
        assert!(!valid_device_id(""));
        assert!(!valid_device_id("not-hex"));
        assert!(!valid_device_id(&"a".repeat(MAX_DEVICE_ID_LEN + 1)));
    }

    #[test]
    fn shared_resets_must_describe_one_consistent_window() {
        let mut reset = SharedResetEvent {
            provider: "codex".into(),
            window_kind: crate::domain::LimitKind::Primary,
            window_duration_mins: 300,
            anchored_at: 1_800_000_000,
            new_resets_at: 1_800_018_000,
            previous_resets_at: 1_800_003_600,
            used_percent_before: 82.0,
            early_by_seconds: 3_600,
            classification: crate::domain::ResetClassification::Scheduled,
            source: "live".into(),
            detected_at: "2027-01-15T08:00:00Z".into(),
        };
        assert!(check_resets(&[reset.clone()]).is_ok());

        reset.new_resets_at += 1;
        assert!(check_resets(&[reset]).is_err());
    }
}
