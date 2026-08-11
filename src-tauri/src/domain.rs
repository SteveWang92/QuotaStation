use serde::{Deserialize, Serialize};

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
    fn unavailable() -> Self {
        Self {
            level: CompactStatusLevel::Unavailable,
            label: "Provider unavailable".to_string(),
            message: "No current Codex quota data is available.".to_string(),
            color: "#ff7469".to_string(),
        }
    }
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
    pub provider: String,
    pub plan_type: Option<String>,
    pub limits: Vec<LimitWindow>,
    pub earned_reset_count: Option<u64>,
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

impl Default for ProviderSnapshot {
    fn default() -> Self {
        Self {
            provider: "codex".to_string(),
            plan_type: None,
            limits: Vec::new(),
            earned_reset_count: None,
            today: TokenUsage::default(),
            api_equivalent_cost_usd: None,
            models: Vec::new(),
            freshness: Freshness::Unavailable,
            stale_age_seconds: None,
            compact_status: CompactStatus::unavailable(),
            last_attempt_at: None,
            last_success_at: None,
            live_error: None,
            history_error: None,
            parser_revision: CCUSAGE_REVISION.to_string(),
            pricing_catalog_revision: PRICING_CATALOG_REVISION.to_string(),
        }
    }
}

impl ProviderSnapshot {
    pub fn update_compact_status(&mut self) {
        self.stale_age_seconds = self.last_success_at.as_deref().and_then(age_seconds);
        self.compact_status = if self.freshness == Freshness::Unavailable || self.limits.is_empty() {
            CompactStatus::unavailable()
        } else if self.freshness == Freshness::Stale {
            CompactStatus {
                level: CompactStatusLevel::Stale,
                label: "Data stale".to_string(),
                message: match self.stale_age_seconds {
                    Some(age) => format!("Last successful update was {} ago.", format_age(age)),
                    None => "The last successful update time is unknown.".to_string(),
                },
                color: "#f0b84b".to_string(),
            }
        } else {
            let minimum = self.limits.iter().filter_map(|limit| limit.remaining_percent).reduce(f64::min);
            match minimum {
                Some(value) if value <= 10.0 => CompactStatus {
                    level: CompactStatusLevel::Critical,
                    label: "Quota critical".to_string(),
                    message: "A Codex quota window has 10% or less remaining.".to_string(),
                    color: "#ff7469".to_string(),
                },
                Some(value) if value <= 30.0 => CompactStatus {
                    level: CompactStatusLevel::Warning,
                    label: "Quota running low".to_string(),
                    message: "A Codex quota window has 30% or less remaining.".to_string(),
                    color: "#f0b84b".to_string(),
                },
                Some(_) => CompactStatus {
                    level: CompactStatusLevel::Healthy,
                    label: "Quota healthy".to_string(),
                    message: "Codex quota and local history are current.".to_string(),
                    color: "#b5e835".to_string(),
                },
                None => CompactStatus::unavailable(),
            }
        };
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
            ..ProviderSnapshot::default()
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
