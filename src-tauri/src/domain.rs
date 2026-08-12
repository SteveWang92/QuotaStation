use serde::{Deserialize, Serialize};

use crate::providers::ProviderKind;

pub const CCUSAGE_REVISION: &str = "033c1f7631f603fc939fdc85163e8203f0084f83";
pub const PRICING_CATALOG_REVISION: &str = env!("QUOTASTATION_PRICING_REVISION");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompactStatusLevel {
    Healthy,
    Warning,
    Critical,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactStatus {
    pub level: CompactStatusLevel,
    pub label: String,
    pub message: String,
    pub color: String,
}

impl CompactStatus {
    fn unavailable(provider: &str) -> Self {
        Self {
            level: CompactStatusLevel::Unavailable,
            label: "Provider unavailable".to_string(),
            message: format!("No current {provider} quota data is available."),
            color: "#ff7469".to_string(),
        }
    }
}

impl CompactStatusLevel {
    /// How loudly a level asks to be looked at, so one aggregate status can stand in for
    /// several providers. A provider that cannot be read at all outranks one that is
    /// merely running low, because only the first needs the user to go and fix something.
    fn severity(self) -> u8 {
        match self {
            CompactStatusLevel::Healthy => 0,
            CompactStatusLevel::Stale => 1,
            CompactStatusLevel::Warning => 2,
            CompactStatusLevel::Unavailable => 3,
            CompactStatusLevel::Critical => 4,
        }
    }
}

/// The single status the tray icon, the taskbar accent, and the panel header show when
/// several providers are on screen at once. It is the loudest provider's own status, so
/// the wording still names which provider raised it.
pub fn aggregate_status(snapshots: &[ProviderSnapshot]) -> CompactStatus {
    snapshots
        .iter()
        .map(|snapshot| &snapshot.compact_status)
        .max_by_key(|status| status.level.severity())
        .cloned()
        .unwrap_or_else(|| CompactStatus::unavailable("provider"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LimitKind {
    Primary,
    Secondary,
}

impl LimitKind {
    /// Live acquisition and restored snapshots share this naming so every surface
    /// describes the same quota window identically.
    pub fn window_label(self, window_duration_mins: Option<i64>) -> String {
        match window_duration_mins {
            Some(300) => "5-hour window".to_string(),
            Some(10_080) => "Weekly window".to_string(),
            Some(value) if value % 1_440 == 0 => format!("{}-day window", value / 1_440),
            Some(value) if value % 60 == 0 => format!("{}-hour window", value / 60),
            _ => match self {
                LimitKind::Primary => "Primary window",
                LimitKind::Secondary => "Secondary window",
            }
            .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub kind: LimitKind,
    pub label: String,
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResetClassification {
    /// The window restarted at, or close enough to, the expiry Codex had published.
    Scheduled,
    /// The server restarted the window well before its published expiry, discarding the
    /// unused remainder and moving the next expiry later.
    Unplanned,
}

impl ResetClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            ResetClassification::Scheduled => "scheduled",
            ResetClassification::Unplanned => "unplanned",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitResetEvent {
    pub window_kind: LimitKind,
    pub window_label: String,
    pub window_duration_mins: i64,
    /// When the restarted window began, recovered from its new expiry.
    pub anchored_at: i64,
    pub new_resets_at: i64,
    pub previous_resets_at: i64,
    pub used_percent_before: f64,
    pub early_by_seconds: i64,
    pub classification: ResetClassification,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input: u64,
    pub cache_read: u64,
    pub output: u64,
    pub reasoning: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub tokens: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub provider: ProviderKind,
    pub display_name: String,
    pub plan_type: Option<String>,
    pub limits: Vec<LimitWindow>,
    pub earned_reset_count: Option<u64>,
    /// Most recent restarts first, so a surface can both annotate the window currently
    /// running and list the ones before it.
    pub recent_resets: Vec<LimitResetEvent>,
    pub today: TokenUsage,
    pub api_equivalent_cost_usd: Option<f64>,
    pub models: Vec<ModelUsage>,
    pub freshness: Freshness,
    pub stale_age_seconds: Option<u64>,
    pub compact_status: CompactStatus,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub live_error: Option<String>,
    pub history_error: Option<String>,
    pub parser_revision: String,
    pub pricing_catalog_revision: String,
}

impl ProviderSnapshot {
    pub fn new(provider: ProviderKind) -> Self {
        Self {
            provider,
            display_name: provider.display_name().to_string(),
            plan_type: None,
            limits: Vec::new(),
            earned_reset_count: None,
            recent_resets: Vec::new(),
            today: TokenUsage::default(),
            api_equivalent_cost_usd: None,
            models: Vec::new(),
            freshness: Freshness::Unavailable,
            stale_age_seconds: None,
            compact_status: CompactStatus::unavailable(provider.display_name()),
            last_attempt_at: None,
            last_success_at: None,
            live_error: None,
            history_error: None,
            parser_revision: CCUSAGE_REVISION.to_string(),
            pricing_catalog_revision: PRICING_CATALOG_REVISION.to_string(),
        }
    }

    /// Freshness follows from the two acquisition paths and the last success, so it is
    /// derived in one place rather than by whoever last wrote to the snapshot.
    pub fn resolve_derived_state(&mut self) {
        self.freshness = match (
            &self.live_error,
            &self.history_error,
            self.last_success_at.is_some(),
        ) {
            (None, None, true) => Freshness::Fresh,
            (_, _, true) => Freshness::Stale,
            _ => Freshness::Unavailable,
        };
        self.update_compact_status();
    }

    pub fn update_compact_status(&mut self) {
        let provider = self.display_name.clone();
        self.stale_age_seconds = self.last_success_at.as_deref().and_then(age_seconds);
        self.compact_status = if self.freshness == Freshness::Unavailable || self.limits.is_empty() {
            CompactStatus::unavailable(&provider)
        } else if self.freshness == Freshness::Stale {
            CompactStatus {
                level: CompactStatusLevel::Stale,
                label: "Data stale".to_string(),
                message: match self.stale_age_seconds {
                    Some(age) => format!("{provider}'s last successful update was {} ago.", format_age(age)),
                    None => format!("{provider}'s last successful update time is unknown."),
                },
                color: "#f0b84b".to_string(),
            }
        } else {
            let minimum = self.limits.iter().filter_map(|limit| limit.remaining_percent).reduce(f64::min);
            match minimum {
                Some(value) if value <= 10.0 => CompactStatus {
                    level: CompactStatusLevel::Critical,
                    label: "Quota critical".to_string(),
                    message: format!("A {provider} quota window has 10% or less remaining."),
                    color: "#ff7469".to_string(),
                },
                Some(value) if value <= 30.0 => CompactStatus {
                    level: CompactStatusLevel::Warning,
                    label: "Quota running low".to_string(),
                    message: format!("A {provider} quota window has 30% or less remaining."),
                    color: "#f0b84b".to_string(),
                },
                Some(_) => CompactStatus {
                    level: CompactStatusLevel::Healthy,
                    label: "Quota healthy".to_string(),
                    message: format!("{provider} quota and local history are current."),
                    color: "#b5e835".to_string(),
                },
                None => CompactStatus::unavailable(&provider),
            }
        };
    }
}

/// Every provider in one payload. The surfaces show them side by side, so asking for
/// them together keeps a refresh to a single round trip and stops two providers being
/// drawn from different moments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub providers: Vec<ProviderSnapshot>,
    pub aggregate: CompactStatus,
}

impl WorkspaceSnapshot {
    pub fn new(providers: Vec<ProviderSnapshot>) -> Self {
        let aggregate = aggregate_status(&providers);
        Self { providers, aggregate }
    }
}

fn age_seconds(value: &str) -> Option<u64> {
    let observed = value.parse::<jiff::Timestamp>().ok()?;
    let elapsed = jiff::Timestamp::now().duration_since(observed);
    Some(elapsed.as_secs().max(0) as u64)
}

fn format_age(seconds: u64) -> String {
    if seconds < 60 { return "less than a minute".to_string(); }
    if seconds < 3_600 { return format!("{}m", seconds / 60); }
    if seconds < 86_400 { return format!("{}h {}m", seconds / 3_600, seconds % 3_600 / 60); }
    format!("{}d {}h", seconds / 86_400, seconds % 86_400 / 3_600)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_snapshot(remaining_percent: f64) -> ProviderSnapshot {
        provider_snapshot(ProviderKind::Codex, remaining_percent)
    }

    fn provider_snapshot(provider: ProviderKind, remaining_percent: f64) -> ProviderSnapshot {
        let mut snapshot = ProviderSnapshot {
            freshness: Freshness::Fresh,
            limits: vec![LimitWindow {
                kind: LimitKind::Primary,
                label: "Primary window".to_string(),
                used_percent: Some(100.0 - remaining_percent),
                remaining_percent: Some(remaining_percent),
                window_duration_mins: Some(300),
                resets_at: None,
            }],
            ..ProviderSnapshot::new(provider)
        };
        snapshot.update_compact_status();
        snapshot
    }

    #[test]
    fn window_labels_describe_the_server_reported_duration() {
        assert_eq!(LimitKind::Primary.window_label(Some(300)), "5-hour window");
        assert_eq!(LimitKind::Secondary.window_label(Some(10_080)), "Weekly window");
        assert_eq!(LimitKind::Secondary.window_label(Some(2_880)), "2-day window");
        assert_eq!(LimitKind::Primary.window_label(None), "Primary window");
    }

    #[test]
    fn compact_status_uses_shared_quota_thresholds() {
        assert_eq!(fresh_snapshot(31.0).compact_status.level, CompactStatusLevel::Healthy);
        assert_eq!(fresh_snapshot(30.0).compact_status.level, CompactStatusLevel::Warning);
        assert_eq!(fresh_snapshot(10.0).compact_status.level, CompactStatusLevel::Critical);
    }

    #[test]
    fn stale_status_takes_priority_over_quota_thresholds() {
        let mut snapshot = fresh_snapshot(5.0);
        snapshot.freshness = Freshness::Stale;
        snapshot.update_compact_status();
        assert_eq!(snapshot.compact_status.level, CompactStatusLevel::Stale);
    }

    #[test]
    fn compact_status_names_the_provider_that_raised_it() {
        assert!(fresh_snapshot(5.0).compact_status.message.contains("Codex"));
    }

    #[test]
    fn the_aggregate_reports_the_loudest_provider() {
        let healthy = provider_snapshot(ProviderKind::Codex, 80.0);
        let critical = provider_snapshot(ProviderKind::Codex, 5.0);
        let aggregate = aggregate_status(&[healthy.clone(), critical]);
        assert_eq!(aggregate.level, CompactStatusLevel::Critical);

        // An unreadable provider outranks one that is merely running low.
        let mut unavailable = provider_snapshot(ProviderKind::Codex, 80.0);
        unavailable.freshness = Freshness::Unavailable;
        unavailable.update_compact_status();
        let warning = provider_snapshot(ProviderKind::Codex, 20.0);
        assert_eq!(
            aggregate_status(&[warning, unavailable]).level,
            CompactStatusLevel::Unavailable
        );

        assert_eq!(
            aggregate_status(&[healthy]).level,
            CompactStatusLevel::Healthy
        );
    }

    #[test]
    fn an_empty_workspace_still_reports_a_status() {
        assert_eq!(
            WorkspaceSnapshot::new(Vec::new()).aggregate.level,
            CompactStatusLevel::Unavailable
        );
    }
}

#[derive(Debug, Clone)]
pub struct HistorySnapshot {
    pub days: Vec<HistoryDay>,
}

#[derive(Debug, Clone)]
pub struct HistoryDay {
    pub date: String,
    pub usage: TokenUsage,
    pub models: Vec<ModelUsage>,
    pub cost_usd: f64,
    pub model_rows: Vec<DailyModelUsage>,
}

#[derive(Debug, Clone)]
pub struct DailyModelUsage {
    pub model: String,
    pub input: u64,
    pub cache_read: u64,
    pub output: u64,
    pub reasoning: u64,
    pub total: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsagePoint {
    pub date: String,
    pub usage: TokenUsage,
    pub api_equivalent_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRangeSnapshot {
    pub start_date: String,
    pub end_date: String,
    pub usage: TokenUsage,
    pub api_equivalent_cost_usd: Option<f64>,
    pub models: Vec<ModelUsage>,
    pub days: Vec<DailyUsagePoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionDiagnostics {
    pub acquisition_path: String,
    pub label: String,
    pub status: String,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherDiagnostics {
    pub status: String,
    pub watched_location_count: usize,
    pub last_event_at: Option<String>,
    pub error: Option<String>,
}

impl Default for WatcherDiagnostics {
    fn default() -> Self {
        Self {
            status: "starting".to_string(),
            watched_location_count: 0,
            last_event_at: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSnapshot {
    pub watcher: WatcherDiagnostics,
    pub acquisitions: Vec<AcquisitionDiagnostics>,
    pub retention: RetentionDiagnostics,
    pub parser_revision: String,
    pub pricing_catalog_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionDiagnostics {
    pub status: String,
    pub last_completed_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LiveSnapshot {
    pub plan_type: Option<String>,
    pub limits: Vec<LimitWindow>,
    pub earned_reset_count: Option<u64>,
}
