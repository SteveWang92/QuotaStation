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
    #[serde(default = "enabled")]
    pub taskbar_widget_enabled: bool,
    #[serde(default)]
    pub status_line_provider_labels: ProviderLabelStyle,
    /// Whether the status line carries the session and the other providers, or only what
    /// Claude Code already knows about itself.
    #[serde(default = "enabled")]
    pub status_line_full_details: bool,
}

fn enabled() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            taskbar_widget_enabled: enabled(),
            status_line_provider_labels: ProviderLabelStyle::default(),
            status_line_full_details: enabled(),
        }
    }
}

/// Anything unreadable is answered with the defaults: a preference that cannot be read is
/// a preference not yet expressed, never a reason to stop.
pub fn load(path: &Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("quotastation-{name}-settings.json"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(
            path.with_extension(format!("json.{}.tmp", std::process::id())),
        );
        path
    }

    #[test]
    fn a_file_written_by_an_earlier_build_keeps_the_choices_it_recorded() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"taskbarWidgetEnabled":false}"#).expect("read old settings");
        assert!(!settings.taskbar_widget_enabled, "the recorded choice survives");
        assert_eq!(settings.status_line_provider_labels, ProviderLabelStyle::Short);
        assert!(settings.status_line_full_details, "an unrecorded choice takes its default");
    }

    #[test]
    fn settings_survive_a_round_trip_through_the_file_format() {
        let settings = AppSettings {
            taskbar_widget_enabled: false,
            status_line_provider_labels: ProviderLabelStyle::Full,
            status_line_full_details: false,
        };
        let encoded = serde_json::to_string(&settings).expect("encode");
        assert_eq!(serde_json::from_str::<AppSettings>(&encoded).expect("decode"), settings);
    }

    #[test]
    fn replacing_the_settings_file_keeps_the_last_complete_record() {
        let path = scratch("atomic");
        save(&path, &AppSettings::default()).expect("write the first settings");
        let expected = AppSettings {
            taskbar_widget_enabled: false,
            status_line_provider_labels: ProviderLabelStyle::Full,
            status_line_full_details: false,
        };
        save(&path, &expected).expect("replace the settings");
        assert_eq!(load(&path), expected);
        let _ = std::fs::remove_file(path);
    }
}
