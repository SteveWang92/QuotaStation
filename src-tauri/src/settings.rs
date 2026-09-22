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

/// How much room the quick panel takes for the same readings.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QuickPanelDensity {
    /// A column per provider, each quota window drawn as label, meter and reset time.
    #[default]
    Standard,
    /// One narrow column whatever the provider count, each quota window drawn as the
    /// taskbar widget's single badge/bar/reading row.
    Compact,
}

/// What the Claude Code status line shows, where, and how.
///
/// Segments are named by string rather than by an enum so that a file naming a segment this
/// build does not know — written by a later version, or by hand — still loads every other
/// preference in it. [`StatusLineLayout::normalized`] reconciles the list with what this
/// build can draw.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusLineLayout {
    /// Every segment this build knows, in the order it is drawn within its row.
    pub segments: Vec<SegmentPlacement>,
    #[serde(default)]
    pub quota: QuotaFormat,
    #[serde(default)]
    pub separators: SeparatorStyle,
    #[serde(default)]
    pub colour: ColourMode,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentPlacement {
    pub id: String,
    pub enabled: bool,
    /// One of the rows, numbered from 1.
    pub row: u8,
}

/// Which parts of each quota window are printed.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaFormat {
    pub used: bool,
    pub remaining: bool,
    pub countdown: bool,
    pub pace: bool,
    pub bar: bool,
}

impl Default for QuotaFormat {
    fn default() -> Self {
        Self { used: true, remaining: false, countdown: true, pace: true, bar: false }
    }
}

/// The marks between segments: one inside a group of related readings, one between groups.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SeparatorStyle {
    /// ` · ` and ` | `.
    #[default]
    Classic,
    /// ` · ` and ` › `.
    Arrow,
    /// ` · ` and the Powerline thin arrow, which needs a Powerline or Nerd Font.
    Powerline,
}

/// Which readings carry a threshold colour.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ColourMode {
    /// Quota and context shares.
    #[default]
    Full,
    QuotaOnly,
    None,
}

pub const STATUS_LINE_ROWS: u8 = 3;

/// Every segment but the quotas, with the row it takes by default: the first says what the
/// session and the project are, the second what Claude Code reported about this session,
/// and the third what QuotaStation itself counted, after the quotas that open it.
const SEGMENTS: [(&str, u8); 25] = [
    ("model", 1),
    ("mode", 1),
    ("sessionName", 1),
    ("agent", 1),
    ("outputStyle", 1),
    ("vimMode", 1),
    ("version", 1),
    ("directory", 1),
    ("branch", 1),
    ("gitOperation", 1),
    ("stash", 1),
    ("lastCommit", 1),
    ("tag", 1),
    ("pullRequest", 1),
    ("context", 2),
    ("largeContext", 2),
    ("cache", 2),
    ("sessionCost", 2),
    ("linesChanged", 2),
    ("duration", 2),
    ("today", 3),
    ("week", 3),
    ("lastReset", 3),
    ("resetCredits", 3),
    ("quotaAge", 3),
];

/// What the line showed before it was configurable. Everything else starts switched off, so
/// an existing installation renders unchanged until the user asks for more.
const ON_BY_DEFAULT: [&str; 8] =
    ["model", "mode", "directory", "branch", "pullRequest", "context", "cache", "sessionCost"];

pub fn quota_segment_id(provider: crate::providers::ProviderKind) -> String {
    format!("quota:{}", provider.key())
}

impl Default for StatusLineLayout {
    fn default() -> Self {
        Self::legacy(true, true)
    }
}

impl StatusLineLayout {
    /// The layout the two switches that came before it described. Both on is the default
    /// line: the session on two rows and every provider's quota on the third. Without the
    /// detail only the model and the quota are left, on one row.
    pub(crate) fn legacy(extra_details: bool, other_providers: bool) -> Self {
        use crate::providers::ProviderKind;
        let placed = |(id, row): &(&str, u8)| SegmentPlacement {
            id: id.to_string(),
            enabled: (extra_details || *id == "model") && ON_BY_DEFAULT.contains(id),
            row: if extra_details { *row } else { 1 },
        };
        let session = SEGMENTS.iter().filter(|(_, row)| *row < STATUS_LINE_ROWS).map(placed);
        let counted = SEGMENTS.iter().filter(|(_, row)| *row == STATUS_LINE_ROWS).map(placed);
        // This client's own quota first, as it always has been.
        let providers = std::iter::once(ProviderKind::Claude)
            .chain(ProviderKind::ALL.into_iter().filter(|kind| *kind != ProviderKind::Claude));
        let quotas = providers.map(|kind| SegmentPlacement {
            id: quota_segment_id(kind),
            enabled: other_providers || kind == ProviderKind::Claude,
            row: if extra_details { STATUS_LINE_ROWS } else { 1 },
        });
        Self {
            segments: session.chain(quotas).chain(counted).collect(),
            quota: QuotaFormat::default(),
            separators: SeparatorStyle::default(),
            colour: ColourMode::default(),
        }
    }

    /// The stored layout reconciled with this build: unknown and repeated segments dropped,
    /// rows kept in range, and a segment the file never mentioned added switched off, so
    /// the settings page can still offer it.
    pub fn normalized(mut self) -> Self {
        let known = Self::default().segments;
        let mut seen = std::collections::HashSet::new();
        self.segments.retain(|segment| {
            known.iter().any(|known| known.id == segment.id) && seen.insert(segment.id.clone())
        });
        for segment in &mut self.segments {
            segment.row = segment.row.clamp(1, STATUS_LINE_ROWS);
        }
        for missing in known {
            if !seen.contains(&missing.id) {
                self.segments.push(SegmentPlacement { enabled: false, ..missing });
            }
        }
        self
    }
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
    pub quick_panel_density: QuickPanelDensity,
    #[serde(default)]
    pub status_line_provider_labels: ProviderLabelStyle,
    #[serde(default)]
    pub status_line_layout: StatusLineLayout,
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
    /// The IANA zone every hour and day bucket and every displayed time follows; unset
    /// follows Windows. See [`crate::clock`].
    #[serde(default)]
    pub time_zone: Option<String>,
    /// Whether this computer's clock is checked against internet time. Off unless chosen,
    /// because it is the one request QuotaStation would send of its own.
    #[serde(default)]
    pub clock_check: bool,
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
            quick_panel_density: QuickPanelDensity::default(),
            status_line_provider_labels: ProviderLabelStyle::default(),
            status_line_layout: StatusLineLayout::default(),
            notify_low_quota: enabled(),
            notify_read_failures: enabled(),
            notify_quota_resets: enabled(),
            device_id: None,
            device_name: None,
            dismissed_reset_notices: Vec::new(),
            quota_disabled_providers: Vec::new(),
            shared_usage_folder: None,
            time_zone: None,
            clock_check: false,
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
            // A file without a layout may still carry the switches that came before it —
            // `statusLineFullDetails`, or the pair that replaced it — and a choice expressed
            // there is kept.
            if stored.get("statusLineLayout").is_none() {
                let switch = |key: &str| stored.get(key).and_then(serde_json::Value::as_bool);
                let full = switch("statusLineFullDetails").unwrap_or(true);
                settings.status_line_layout = StatusLineLayout::legacy(
                    switch("statusLineExtraDetails").unwrap_or(full),
                    switch("statusLineOtherProviders").unwrap_or(full),
                );
            }
            settings.status_line_layout = settings.status_line_layout.normalized();
            // A zone name this build does not know — edited by hand, or dropped from the
            // zone database — cannot be honoured, and is read as no choice made.
            if settings
                .time_zone
                .as_deref()
                .is_some_and(|name| crate::clock::resolve(name).is_err())
            {
                settings.time_zone = None;
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
    let content = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    // The status-line process reads this file while the application is running. Publish a
    // complete replacement so it never sees a truncated JSON document, and a failed write
    // leaves the last saved preferences intact.
    crate::fs_atomic::write(path, content).map_err(|error| error.to_string())?;
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
        assert_eq!(settings.status_line_layout, StatusLineLayout::default());
        assert!(settings.notify_low_quota, "an unrecorded choice takes its default");
        assert!(settings.notify_read_failures, "an unrecorded choice takes its default");
        assert!(settings.notify_quota_resets, "an unrecorded choice takes its default");
    }

    fn enabled_ids(layout: &StatusLineLayout) -> Vec<(&str, u8)> {
        layout
            .segments
            .iter()
            .filter(|segment| segment.enabled)
            .map(|segment| (segment.id.as_str(), segment.row))
            .collect()
    }

    #[test]
    fn the_old_status_line_switches_become_the_layout_they_described() {
        let path = scratch("legacy-status-line-details");
        std::fs::write(&path, r#"{"statusLineFullDetails":false}"#).expect("write old settings");
        assert_eq!(
            enabled_ids(&load(&path).status_line_layout),
            [("model", 1), ("quota:claude", 1)]
        );

        std::fs::write(
            &path,
            r#"{"statusLineExtraDetails":false,"statusLineOtherProviders":true}"#,
        )
        .expect("write old settings");
        assert_eq!(
            enabled_ids(&load(&path).status_line_layout),
            [("model", 1), ("quota:claude", 1), ("quota:codex", 1)]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_hand_edited_layout_is_reconciled_with_what_this_build_can_draw() {
        let path = scratch("edited-layout");
        std::fs::write(
            &path,
            r#"{"statusLineLayout":{"segments":[
                {"id":"quota:codex","enabled":true,"row":9},
                {"id":"weather","enabled":true,"row":1},
                {"id":"model","enabled":true,"row":1},
                {"id":"model","enabled":false,"row":2}
            ]}}"#,
        )
        .expect("write edited settings");
        let layout = load(&path).status_line_layout;
        assert_eq!(enabled_ids(&layout), [("quota:codex", STATUS_LINE_ROWS), ("model", 1)]);
        assert_eq!(
            layout.segments.len(),
            StatusLineLayout::default().segments.len(),
            "every known segment is still offered"
        );
        let _ = std::fs::remove_file(path);
    }

    /// Every field moved away from its default, so a field the file format drops shows up.
    fn customised() -> AppSettings {
        AppSettings {
            theme: crate::theme::ThemePreference::Light,
            taskbar_widget_enabled: false,
            taskbar_widget_display: Some("\\\\.\\DISPLAY2".to_string()),
            quick_panel_density: QuickPanelDensity::Compact,
            status_line_provider_labels: ProviderLabelStyle::Full,
            status_line_layout: StatusLineLayout {
                quota: QuotaFormat { bar: true, ..QuotaFormat::default() },
                separators: SeparatorStyle::Powerline,
                colour: ColourMode::QuotaOnly,
                ..StatusLineLayout::legacy(false, false)
            },
            notify_low_quota: false,
            notify_read_failures: false,
            notify_quota_resets: false,
            device_id: Some("18f3c".to_string()),
            device_name: Some("Workshop".to_string()),
            dismissed_reset_notices: vec!["codex:primary:1781654400".to_string()],
            quota_disabled_providers: vec!["codex".to_string()],
            shared_usage_folder: Some("D:\\Sync\\QuotaStation".to_string()),
            time_zone: Some("Europe/London".to_string()),
            clock_check: true,
        }
    }

    #[test]
    fn an_unknown_time_zone_in_the_file_is_read_as_no_choice() {
        let path = scratch("unknown-zone");
        std::fs::write(&path, r#"{"timeZone":"Mars/Olympus_Mons","taskbarWidgetEnabled":false}"#)
            .expect("write edited settings");
        let settings = load(&path);
        assert_eq!(settings.time_zone, None);
        assert!(!settings.taskbar_widget_enabled, "every other preference is kept");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn settings_survive_a_round_trip_through_the_file_format() {
        let settings = customised();
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
        let expected = customised();
        save(&path, &expected).expect("replace the settings");
        assert_eq!(load(&path), expected);
        let _ = std::fs::remove_file(path);
    }
}
