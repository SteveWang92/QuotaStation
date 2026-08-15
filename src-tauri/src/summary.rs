//! A small quota summary on disk, for readers that are not the running application.
//!
//! The status-line bridge is a process that lives for a few milliseconds inside Claude
//! Code. It has no handle on the running application, no database connection, and no time
//! to acquire either, so it cannot ask what Codex's quota looks like — but that is exactly
//! what makes its one row of screen worth spending. The application therefore leaves the
//! answer where a process with no context can pick it up: one file, written whenever a
//! snapshot is published, read by whoever needs it.
//!
//! What goes in is the normalized quota vocabulary and nothing else — percentages, restart
//! times, and today's totals. No paths, no session content, no credentials, exactly as with
//! the activity log this sits beside. The file never leaves the application data directory.

use serde::{Deserialize, Serialize};

use crate::domain::{LimitKind, WorkspaceSnapshot};
use crate::providers::claude::statusline::app_data_dir;

const SUMMARY_FILE: &str = "quota-summary.json";

/// The shape the reader expects. A reader that finds anything else stops rather than
/// guessing, so an older build never renders a newer file as though it understood it.
const SCHEMA: u32 = 1;

/// How long a summary describes the present. The application republishes on every refresh,
/// so anything older than this means it is not running — and a quota read a quarter of an
/// hour ago is a number worth omitting rather than one worth showing without saying so.
const MAX_AGE_SECS: i64 = 15 * 60;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSummary {
    pub schema: u32,
    pub generated_at: i64,
    pub providers: Vec<ProviderQuota>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuota {
    /// The database's provider key, so a reader can tell the providers apart.
    pub provider: String,
    /// Both names, because the reader's own setting decides which of them it draws.
    pub short_name: String,
    pub display_name: String,
    pub windows: Vec<QuotaWindow>,
    pub today_tokens: u64,
    pub api_equivalent_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// The window's duration in the shortest form that still names it, such as `5h`.
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}

/// The same duration vocabulary the taskbar badge uses, in lower case: `5h`, `7d`.
fn short_window_label(duration_mins: Option<i64>, kind: LimitKind) -> String {
    match duration_mins {
        Some(value) if value % 1_440 == 0 => format!("{}d", value / 1_440),
        Some(value) if value % 60 == 0 => format!("{}h", value / 60),
        Some(value) => format!("{value}m"),
        None => match kind {
            LimitKind::Primary => "primary".to_string(),
            LimitKind::Secondary => "secondary".to_string(),
        },
    }
}

fn summarize(workspace: &WorkspaceSnapshot, now: i64) -> QuotaSummary {
    QuotaSummary {
        schema: SCHEMA,
        generated_at: now,
        providers: workspace
            .providers
            .iter()
            .map(|snapshot| ProviderQuota {
                provider: snapshot.provider.key().to_string(),
                short_name: snapshot.short_name.clone(),
                display_name: snapshot.display_name.clone(),
                windows: snapshot
                    .limits
                    .iter()
                    // A window with no percentage says nothing a reader could render, and
                    // the reader has no room to explain an absence.
                    .filter_map(|limit| {
                        Some(QuotaWindow {
                            label: short_window_label(limit.window_duration_mins, limit.kind),
                            used_percent: limit.used_percent?,
                            resets_at: limit.resets_at,
                        })
                    })
                    .collect(),
                today_tokens: snapshot.today.total,
                api_equivalent_cost_usd: snapshot.api_equivalent_cost_usd,
            })
            .collect(),
    }
}

fn summary_path() -> Option<std::path::PathBuf> {
    app_data_dir().map(|dir| dir.join(SUMMARY_FILE))
}

/// Records the current snapshot for readers outside this process. Failures are silent: a
/// summary that cannot be written must never become a second fault to report, and the
/// application's own surfaces already have the data.
pub fn publish(workspace: &WorkspaceSnapshot) {
    let summary = summarize(workspace, jiff::Timestamp::now().as_second());
    let Some(path) = summary_path() else { return };
    let _ = write_atomically(&path, &summary);
}

fn write_atomically(path: &std::path::Path, summary: &QuotaSummary) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let Ok(content) = serde_json::to_string(summary) else { return Ok(()) };
    // A refresh and a status-line render can coincide, so the file is published by rename:
    // a reader never sees a half-written summary.
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staging, content)?;
    std::fs::rename(&staging, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })
}

/// The published summary, if one describes the present.
///
/// Nothing here is an error worth reporting: a missing file means the application has never
/// run, an old one means it is not running now, and an unreadable one means a build wrote
/// a shape this one does not know. In all three cases the caller has no summary, which is
/// the one thing it needs to be told.
pub fn load_fresh(now: i64) -> Option<QuotaSummary> {
    let content = std::fs::read_to_string(summary_path()?).ok()?;
    let summary: QuotaSummary = serde_json::from_str(&content).ok()?;
    if summary.schema != SCHEMA {
        return None;
    }
    // A summary stamped in the future is a clock that moved, not a fresh reading.
    (summary.generated_at <= now && now - summary.generated_at <= MAX_AGE_SECS).then_some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Freshness, LimitWindow, ProviderSnapshot, WindowSource};
    use crate::providers::ProviderKind;

    fn workspace() -> WorkspaceSnapshot {
        let mut codex = ProviderSnapshot::new(ProviderKind::Codex);
        codex.limits = vec![
            LimitWindow {
                kind: LimitKind::Primary,
                label: "5-hour window".to_string(),
                used_percent: Some(62.0),
                window_duration_mins: Some(300),
                resets_at: Some(1_800_007_800),
                source: WindowSource::AppServer,
                observed_at: 1_800_000_000,
                freshness: Freshness::Fresh,
            },
            // Reported but unreadable: no percentage, so there is nothing to render.
            LimitWindow {
                kind: LimitKind::Secondary,
                label: "Weekly window".to_string(),
                used_percent: None,
                window_duration_mins: Some(10_080),
                resets_at: None,
                source: WindowSource::AppServer,
                observed_at: 1_800_000_000,
                freshness: Freshness::Fresh,
            },
        ];
        codex.today.total = 1_234;
        WorkspaceSnapshot {
            aggregate: crate::domain::aggregate_status(std::slice::from_ref(&codex)),
            providers: vec![codex],
        }
    }

    #[test]
    fn a_summary_carries_the_windows_a_reader_could_draw_and_no_others() {
        let summary = summarize(&workspace(), 1_800_000_000);
        assert_eq!(summary.schema, SCHEMA);
        let codex = &summary.providers[0];
        assert_eq!(codex.short_name, "CDX");
        assert_eq!(codex.windows.len(), 1, "the unreadable window is left out");
        assert_eq!(codex.windows[0].label, "5h");
        assert_eq!(codex.windows[0].used_percent, 62.0);
        assert_eq!(codex.today_tokens, 1_234);
    }

    #[test]
    fn window_labels_use_the_shortest_form_that_still_names_the_duration() {
        assert_eq!(short_window_label(Some(300), LimitKind::Primary), "5h");
        assert_eq!(short_window_label(Some(10_080), LimitKind::Secondary), "7d");
        assert_eq!(short_window_label(Some(90), LimitKind::Primary), "90m");
        assert_eq!(short_window_label(None, LimitKind::Secondary), "secondary");
    }

    #[test]
    fn the_summary_survives_a_round_trip_through_the_file_format() {
        let summary = summarize(&workspace(), 1_800_000_000);
        let encoded = serde_json::to_string(&summary).expect("encode the summary");
        let decoded: QuotaSummary = serde_json::from_str(&encoded).expect("decode the summary");
        assert_eq!(decoded, summary);
    }
}
