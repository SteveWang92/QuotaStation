//! The deliberately small, shareable diagnostic report.
//!
//! This is a whitelist projection of the diagnostics screen, not a copy of the database,
//! settings, or activity log. Keeping that boundary in one type makes it possible to audit
//! what a support file can contain.

use std::path::Path;

use serde::Serialize;

use crate::domain::{
    AcquisitionDiagnostics, DiagnosticsSnapshot, LimitWindow, ProviderSnapshot,
    RetentionDiagnostics, SharedFolderDiagnostics, WatcherDiagnostics,
};

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticExport {
    format_version: u32,
    exported_at: String,
    diagnostics: ExportDiagnostics,
    providers: Vec<ExportProvider>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportDiagnostics {
    watcher: ExportWatcher,
    acquisitions: Vec<ExportAcquisition>,
    retention: ExportRetention,
    shared_folder: ExportSharedFolder,
    parser_revision: String,
    pricing_catalog_revision: String,
    app_version: String,
    build_commit: String,
    build_kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportAcquisition {
    label: String,
    status: String,
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    has_error: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportWatcher {
    status: String,
    watched_location_count: usize,
    last_event_at: Option<String>,
    has_error: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportRetention {
    status: String,
    last_completed_at: Option<String>,
    has_error: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportSharedFolder {
    status: String,
    last_completed_at: Option<String>,
    has_error: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportProvider {
    provider: String,
    limits: Vec<ExportLimit>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportLimit {
    label: String,
    used_percent: Option<f64>,
    resets_at: Option<i64>,
    source: String,
    observed_at: i64,
    freshness: String,
}

impl DiagnosticExport {
    pub fn new(diagnostics: DiagnosticsSnapshot, providers: Vec<ProviderSnapshot>) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            exported_at: jiff::Timestamp::now().to_string(),
            diagnostics: ExportDiagnostics {
                watcher: ExportWatcher::from(&diagnostics.watcher),
                acquisitions: diagnostics
                    .acquisitions
                    .iter()
                    .map(ExportAcquisition::from)
                    .collect(),
                retention: ExportRetention::from(&diagnostics.retention),
                shared_folder: ExportSharedFolder::from(&diagnostics.shared_folder),
                parser_revision: diagnostics.parser_revision,
                pricing_catalog_revision: diagnostics.pricing_catalog_revision,
                app_version: diagnostics.app_version,
                build_commit: diagnostics.build_commit,
                build_kind: diagnostics.build_kind,
            },
            providers: providers
                .into_iter()
                .map(|provider| ExportProvider {
                    provider: provider.provider.key().to_string(),
                    limits: provider.limits.iter().map(ExportLimit::from).collect(),
                })
                .collect(),
        }
    }

    pub fn write_to(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .filter(|parent| parent.is_dir())
            .ok_or_else(|| "Choose an existing folder for the diagnostic export.".to_string())?;
        let content = serde_json::to_vec_pretty(self)
            .map_err(|_| "Diagnostic export could not be prepared.".to_string())?;
        let staging = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name().and_then(|name| name.to_str()).unwrap_or("diagnostics.json"),
            std::process::id()
        ));
        std::fs::write(&staging, content)
            .map_err(|_| "Diagnostic export could not be written.".to_string())?;
        // The rename replaces an existing export in one step. A failure leaves that file
        // untouched, so the staging copy is what goes.
        if std::fs::rename(&staging, path).is_err() {
            let _ = std::fs::remove_file(&staging);
            return Err("Diagnostic export could not be saved.".to_string());
        }
        Ok(())
    }
}

impl From<&AcquisitionDiagnostics> for ExportAcquisition {
    fn from(diagnostics: &AcquisitionDiagnostics) -> Self {
        Self {
            label: diagnostics.label.clone(),
            status: diagnostics.status.clone(),
            last_attempt_at: diagnostics.last_attempt_at.clone(),
            last_success_at: diagnostics.last_success_at.clone(),
            has_error: diagnostics.error.is_some(),
        }
    }
}

impl From<&WatcherDiagnostics> for ExportWatcher {
    fn from(diagnostics: &WatcherDiagnostics) -> Self {
        Self {
            status: diagnostics.status.clone(),
            watched_location_count: diagnostics.watched_location_count,
            last_event_at: diagnostics.last_event_at.clone(),
            has_error: diagnostics.error.is_some(),
        }
    }
}

impl From<&RetentionDiagnostics> for ExportRetention {
    fn from(diagnostics: &RetentionDiagnostics) -> Self {
        Self {
            status: diagnostics.status.clone(),
            last_completed_at: diagnostics.last_completed_at.clone(),
            has_error: diagnostics.error.is_some(),
        }
    }
}

impl From<&SharedFolderDiagnostics> for ExportSharedFolder {
    fn from(diagnostics: &SharedFolderDiagnostics) -> Self {
        Self {
            status: diagnostics.status.clone(),
            last_completed_at: diagnostics.last_completed_at.clone(),
            has_error: diagnostics.error.is_some(),
        }
    }
}

impl From<&LimitWindow> for ExportLimit {
    fn from(limit: &LimitWindow) -> Self {
        Self {
            label: limit.label.clone(),
            used_percent: limit.used_percent,
            resets_at: limit.resets_at,
            source: limit.source.as_str().to_string(),
            observed_at: limit.observed_at,
            freshness: format!("{:?}", limit.freshness).to_lowercase(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{domain::DeviceDiagnostics, providers::ProviderKind};

    #[test]
    fn omits_device_identifiers_from_the_serialized_export() {
        let diagnostics = DiagnosticsSnapshot {
            watcher: WatcherDiagnostics::default(),
            acquisitions: Vec::new(),
            retention: RetentionDiagnostics {
                status: "pending".to_string(),
                last_completed_at: None,
                error: Some("Private computer at C:\\Users\\owner".to_string()),
            },
            shared_folder: SharedFolderDiagnostics::default(),
            devices: vec![DeviceDiagnostics {
                id: "secret-device-id".to_string(),
                display_name: "Private computer".to_string(),
                local: true,
                last_import_at: None,
            }],
            parser_revision: "parser".to_string(),
            pricing_catalog_revision: "pricing".to_string(),
            app_version: "0.0.0".to_string(),
            build_commit: "commit".to_string(),
            build_kind: "debug".to_string(),
        };
        let export =
            DiagnosticExport::new(diagnostics, vec![ProviderSnapshot::new(ProviderKind::Codex)]);

        let json = serde_json::to_string(&export).expect("serialize export");
        assert!(!json.contains("secret-device-id"));
        assert!(!json.contains("Private computer"));
        assert!(!json.contains("C:\\Users\\owner"));
        assert!(!json.contains("devices"));
    }
}
