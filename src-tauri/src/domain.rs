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
pub enum LimitKind {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub kind: LimitKind,
    pub label: String,
    pub used_percent: Option<f64>,
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
            last_attempt_at: None,
            last_success_at: None,
            live_error: None,
            history_error: None,
            parser_revision: CCUSAGE_REVISION.to_string(),
            pricing_catalog_revision: PRICING_CATALOG_REVISION.to_string(),
        }
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
