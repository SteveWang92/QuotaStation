//! What the user has chosen, on disk.
//!
//! The status-line bridge is a separate process that starts before Tauri does, so the
//! preferences it obeys cannot live in the running application's state. They live in one
//! file, which the application writes and both processes read.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "settings.json";

/// How a provider is named where the name sits beside a reading rather than above one.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderLabelStyle {
    /// `CDX`, `CLD` — the taskbar widget's vocabulary, for a row already carrying four
    /// readings.
    #[default]
    Short,
    /// `Codex`, `Claude Code`.
    Full,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Which palette every window but the taskbar widget is drawn in. Dark is the default
    /// because it is what QuotaStation looked like before the choice existed; the widget
    /// follows the Windows taskbar instead, which is not a preference anyone expressed.
    #[serde(default)]
    pub theme: crate::theme::ThemePreference,
    #[serde(default = "enabled")]
    pub taskbar_widget_enabled: bool,
    /// Which display's taskbar hosts the status, by Windows device name (`\\.\DISPLAY1`).
    /// Unset — and a name no longer attached — means the primary taskbar, so unplugging a
    /// monitor moves the status rather than losing it.
    #[serde(default)]
    pub taskbar_widget_display: Option<String>,
    #[serde(default)]
    pub status_line_provider_labels: ProviderLabelStyle,
    /// Whether the status line reports every provider QuotaStation watches, or only the
    /// client it is being rendered inside.
    #[serde(default = "enabled")]
    pub status_line_other_providers: bool,
    /// Whether the status line carries the session detail Claude Code's own footer never
    /// shows — the project, the request and the spend. Off leaves the model and the quota.
    #[serde(default = "enabled")]
    pub status_line_extra_details: bool,
    /// Whether a quota window crossing the shared warning or critical share is announced.
    #[serde(default = "enabled")]
    pub notify_low_quota: bool,
    /// Whether a provider that stops answering, or whose data goes stale, is announced.
    #[serde(default = "enabled")]
    pub notify_read_failures: bool,
    /// Whether a confirmed quota window restart is announced.
    #[serde(default = "enabled")]
    pub notify_quota_resets: bool,
    /// How this machine names itself to the others sharing a usage folder. Generated once
    /// and then kept: the rows another machine has stored for it are keyed by this, so a
    /// new identifier would orphan every one of them. Unset until the first start that
    /// wrote one.
    #[serde(default)]
    pub device_id: Option<String>,
    /// What the device split calls this machine. Defaults to the computer name, and is
    /// only a label — renaming the machine renames the rows rather than splitting them.
    #[serde(default)]
    pub device_name: Option<String>,
    /// Which "possibly restarted early" notes the user has acknowledged, as
    /// `provider:windowKind:newResetsAt`. The note explains the expiry the window is
    /// showing right now, so keying it on that expiry is what brings it back at the next
    /// restart without ever bringing back the one already read. The settings page rewrites
    /// the whole list against the windows currently in view, which is what keeps it short.
    #[serde(default)]
    pub dismissed_reset_notices: Vec<String>,
    /// The providers whose quota is not tracked, by
    /// [`crate::providers::ProviderKind::key`]. Nothing starts their client to read a
    /// percentage and no surface draws one, so a client that cannot answer stops being
    /// asked. Their usage history is unaffected: it is parsed from files already on disk
    /// and keeps its place in the charts. Stored as plain keys so a file written by a
    /// later version, naming a provider this one has never heard of, still loads every
    /// other preference in it.
    #[serde(default)]
    pub quota_disabled_providers: Vec<String>,
    /// The folder this machine's aggregates are written to and the other machines' are
    /// read from — whatever folder a sync client already keeps in step. Unset is the
    /// ordinary single-machine case, where nothing is exported or read.
    #[serde(default)]
    pub shared_usage_folder: Option<String>,
}

/// A fresh identity for this machine.
///
/// It has one job: to differ from the other machine's. Two first starts cannot land on the
/// same nanosecond, and the value is written once and then read for good.
pub fn new_device_id() -> String {
    format!("{:x}", jiff::Timestamp::now().as_nanosecond())
}

/// What this machine calls itself before anyone renames it.
pub fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "This machine".to_string())
}

fn enabled() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: crate::theme::ThemePreference::default(),
            taskbar_widget_enabled: enabled(),
            taskbar_widget_display: None,
            status_line_provider_labels: ProviderLabelStyle::default(),
            status_line_other_providers: enabled(),
            status_line_extra_details: enabled(),
            notify_low_quota: enabled(),
            notify_read_failures: enabled(),
            notify_quota_resets: enabled(),
            device_id: None,
            device_name: None,
            dismissed_reset_notices: Vec::new(),
            quota_disabled_providers: Vec::new(),
            shared_usage_folder: None,
        }
    }
}

/// Anything unreadable is answered with the defaults: a preference that cannot be read is
/// a preference not yet expressed, never a reason to stop.
pub fn load(path: &Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| {
            let stored: serde_json::Value = serde_json::from_str(&content).ok()?;
            let mut settings: AppSettings = serde_json::from_value(stored.clone()).ok()?;
            // v0.2 had one switch for both choices v0.3 split apart. Preserve an expressed
            // choice wherever the old file does not yet carry its replacement.
            if let Some(legacy) =
                stored.get("statusLineFullDetails").and_then(serde_json::Value::as_bool)
            {
                if stored.get("statusLineOtherProviders").is_none() {
                    settings.status_line_other_providers = legacy;
                }
                if stored.get("statusLineExtraDetails").is_none() {
                    settings.status_line_extra_details = legacy;
                }
            }
            Some(settings)
        })
        .unwrap_or_default()
}

/// The settings as a process with no Tauri handle can find them.
pub fn load_default() -> AppSettings {
    default_path().map(|path| load(&path)).unwrap_or_default()
}

fn default_path() -> Option<PathBuf> {
    crate::providers::claude::statusline::app_data_dir().map(|dir| dir.join(SETTINGS_FILE))
}

pub fn save(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    // The status-line process reads this file while the application is running. Publish a
    // complete replacement so it never sees a truncated JSON document, and a failed write
    // leaves the last saved preferences intact.
    let staging = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&staging, content).map_err(|error| error.to_string())?;
    std::fs::rename(&staging, path).map_err(|error| {
        let _ = std::fs::remove_file(&staging);
        error.to_string()
    })?;
    remove_abandoned_staging(path);
    Ok(())
}

/// A process killed between the write and the rename leaves its staging file behind, and
/// nothing else would ever collect it. A completed save is the moment to sweep: the
/// application is single-instance and only it writes these names, so anything still here
/// belongs to a run that is over.
fn remove_abandoned_staging(path: &Path) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return;
    };
    let prefix = format!("{name}.");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if file_name.starts_with(&prefix) && file_name.ends_with(".tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("quotastation-{name}-settings.json"));
        let _ = std::fs::remove_file(&path);
        let _ =
            std::fs::remove_file(path.with_extension(format!("json.{}.tmp", std::process::id())));
        path
    }

    #[test]
    fn a_file_written_by_an_earlier_build_keeps_the_choices_it_recorded() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"taskbarWidgetEnabled":false}"#).expect("read old settings");
        assert!(!settings.taskbar_widget_enabled, "the recorded choice survives");
        assert_eq!(
            settings.taskbar_widget_display, None,
            "no display chosen means the primary one"
        );
        assert_eq!(settings.status_line_provider_labels, ProviderLabelStyle::Short);
        assert!(settings.status_line_other_providers, "an unrecorded choice takes its default");
        assert!(settings.status_line_extra_details, "an unrecorded choice takes its default");
        assert!(settings.notify_low_quota, "an unrecorded choice takes its default");
        assert!(settings.notify_read_failures, "an unrecorded choice takes its default");
        assert!(settings.notify_quota_resets, "an unrecorded choice takes its default");
    }

    #[test]
    fn the_old_combined_status_line_choice_migrates_to_both_new_choices() {
        let path = scratch("legacy-status-line-details");
        std::fs::write(&path, r#"{"statusLineFullDetails":false}"#).expect("write old settings");
        let settings = load(&path);
        assert!(!settings.status_line_other_providers);
        assert!(!settings.status_line_extra_details);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn settings_survive_a_round_trip_through_the_file_format() {
        let settings = AppSettings {
            theme: crate::theme::ThemePreference::Light,
            taskbar_widget_enabled: false,
            taskbar_widget_display: Some("\\\\.\\DISPLAY2".to_string()),
            status_line_provider_labels: ProviderLabelStyle::Full,
            status_line_other_providers: false,
            status_line_extra_details: false,
            notify_low_quota: false,
            notify_read_failures: false,
            notify_quota_resets: false,
            device_id: Some("18f3c".to_string()),
            device_name: Some("Workshop".to_string()),
            dismissed_reset_notices: vec!["codex:primary:1781654400".to_string()],
            quota_disabled_providers: vec!["codex".to_string()],
            shared_usage_folder: Some("D:\\Sync\\QuotaStation".to_string()),
        };
        let encoded = serde_json::to_string(&settings).expect("encode");
        assert_eq!(serde_json::from_str::<AppSettings>(&encoded).expect("decode"), settings);
    }

    #[test]
    fn a_save_collects_the_staging_file_a_killed_run_left_behind() {
        let path = scratch("staging");
        let abandoned = path.with_extension("json.4294967295.tmp");
        std::fs::write(&abandoned, "{}").expect("leave a staging file behind");
        save(&path, &AppSettings::default()).expect("write the settings");
        assert!(!abandoned.exists(), "the dead run's staging file is collected");
        assert_eq!(load(&path), AppSettings::default(), "the settings themselves survive");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn replacing_the_settings_file_keeps_the_last_complete_record() {
        let path = scratch("atomic");
        save(&path, &AppSettings::default()).expect("write the first settings");
        let expected = AppSettings {
            theme: crate::theme::ThemePreference::System,
            taskbar_widget_enabled: false,
            taskbar_widget_display: Some("\\\\.\\DISPLAY2".to_string()),
            status_line_provider_labels: ProviderLabelStyle::Full,
            status_line_other_providers: false,
            status_line_extra_details: false,
            notify_low_quota: false,
            notify_read_failures: false,
            notify_quota_resets: false,
            device_id: Some("18f3c".to_string()),
            device_name: Some("Workshop".to_string()),
            dismissed_reset_notices: vec!["codex:primary:1781654400".to_string()],
            quota_disabled_providers: vec!["codex".to_string()],
            shared_usage_folder: Some("D:\\Sync\\QuotaStation".to_string()),
        };
        save(&path, &expected).expect("replace the settings");
        assert_eq!(load(&path), expected);
        let _ = std::fs::remove_file(path);
    }
}
