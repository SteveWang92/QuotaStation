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

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    domain::{CCUSAGE_REVISION, DeviceUsageRow, SharedFolderDiagnostics},
    sanitize::sanitize_error,
    storage::DeviceImport,
};

/// The shape a reader expects. A file written by a newer build is left alone rather than
/// read as though this one understood it.
const FORMAT_VERSION: u32 = 1;

const FILE_PREFIX: &str = "usage-";
const FILE_SUFFIX: &str = ".json";

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
}

fn file_name(device_id: &str) -> String {
    format!("{FILE_PREFIX}{device_id}{FILE_SUFFIX}")
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
    let content = serde_json::to_vec(&ExportFile {
        format_version: FORMAT_VERSION,
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        timezone: system_timezone(),
        parser_revision: CCUSAGE_REVISION.to_string(),
        daily,
        hourly,
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
        if !file_device.chars().all(|character| character.is_ascii_hexdigit()) {
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
    let timezone = system_timezone();
    anyhow::ensure!(
        published.timezone == timezone,
        "aggregated in {} rather than {timezone}",
        published.timezone
    );

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
            },
            &jiff::Timestamp::now().to_string(),
        )
        .await?;
    Ok(())
}
