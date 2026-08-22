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

/// The three colours a reading is drawn in, and the shares of a window that earn them.
/// Every surface reads them from here, so a percentage never means one thing on the
/// dashboard and another in the tray.
pub const HEALTHY_COLOR: &str = "#b5e835";
pub const WARNING_COLOR: &str = "#f0b84b";
pub const CRITICAL_COLOR: &str = "#ff7469";
pub const WARNING_PERCENT: f64 = 70.0;
pub const CRITICAL_PERCENT: f64 = 90.0;

/// The colour one window's own reading earns. A window with no published percentage is
/// tracked rather than in trouble, so it is drawn as healthy.
pub fn quota_color(used_percent: Option<f64>) -> String {
    match used_percent {
        Some(value) if value >= CRITICAL_PERCENT => CRITICAL_COLOR,
        Some(value) if value >= WARNING_PERCENT => WARNING_COLOR,
        _ => HEALTHY_COLOR,
    }
    .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactStatus {
    pub level: CompactStatusLevel,
    pub label: String,
    pub color: String,
}

impl CompactStatus {
    fn unavailable() -> Self {
        Self {
            level: CompactStatusLevel::Unavailable,
            label: "Provider unavailable".to_string(),
            color: CRITICAL_COLOR.to_string(),
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
/// several providers are on screen at once. It is the loudest provider's own status.
pub fn aggregate_status(snapshots: &[ProviderSnapshot]) -> CompactStatus {
    snapshots
        .iter()
        .map(|snapshot| &snapshot.compact_status)
        .max_by_key(|status| status.level.severity())
        .cloned()
        .unwrap_or(CompactStatus::unavailable())
}

/// Declaration order is display order, and the ordering derives make it so wherever windows
/// are collected by kind rather than listed in the order they were read.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum LimitKind {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowSource {
    AppServer,
    SessionLog,
    StatusLine,
}

impl WindowSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppServer => "app_server",
            Self::SessionLog => "session_log",
            Self::StatusLine => "status_line",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "app_server" => Some(Self::AppServer),
            "session_log" => Some(Self::SessionLog),
            "status_line" => Some(Self::StatusLine),
            _ => None,
        }
    }
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
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
    pub source: WindowSource,
    pub observed_at: i64,
    pub freshness: Freshness,
    /// This window's own colour. A provider is as loud as its loudest window, but a window
    /// is only ever as loud as itself: a weekly allowance barely touched must not turn red
    /// because the five-hour one beside it ran out.
    ///
    /// Left empty where windows are read, and filled by `resolve_derived_state` before any
    /// surface sees one — an acquisition path reports a reading, it does not decide how the
    /// reading is drawn.
    #[serde(default)]
    pub status_color: String,
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
    /// The same name in the three characters a crowded row can spare.
    pub short_name: String,
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
    pub last_live_success_at: Option<String>,
    pub last_history_success_at: Option<String>,
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
            short_name: provider.short_name().to_string(),
            plan_type: None,
            limits: Vec::new(),
            earned_reset_count: None,
            recent_resets: Vec::new(),
            today: TokenUsage::default(),
            api_equivalent_cost_usd: None,
            models: Vec::new(),
            freshness: Freshness::Unavailable,
            stale_age_seconds: None,
            compact_status: CompactStatus::unavailable(),
            last_attempt_at: None,
            last_live_success_at: None,
            last_history_success_at: None,
            live_error: None,
            history_error: None,
            parser_revision: CCUSAGE_REVISION.to_string(),
            pricing_catalog_revision: PRICING_CATALOG_REVISION.to_string(),
        }
    }

    /// Quota freshness follows only from live acquisition and each window's own source
    /// timestamp. A successful history parse must never renew an older quota reading.
    pub fn resolve_derived_state(&mut self) {
        let now = jiff::Timestamp::now().as_second();
        for limit in &mut self.limits {
            let age = now.saturating_sub(limit.observed_at);
            let max_age = match limit.source {
                WindowSource::AppServer => {
                    self.provider.live_refresh_interval().as_secs() as i64 * 3
                }
                WindowSource::SessionLog => limit.window_duration_mins.unwrap_or(300) * 60,
                WindowSource::StatusLine => limit.window_duration_mins.unwrap_or(300) * 60 / 5,
            };
            limit.freshness = if age <= max_age { Freshness::Fresh } else { Freshness::Stale };
            limit.status_color = quota_color(limit.used_percent);
        }
        self.freshness = if self.last_live_success_at.is_none() || self.limits.is_empty() {
            Freshness::Unavailable
        } else if self.live_error.is_some()
            || self.limits.iter().any(|limit| limit.freshness == Freshness::Stale)
        {
            Freshness::Stale
        } else {
            Freshness::Fresh
        };
        self.update_compact_status();
    }

    pub fn update_compact_status(&mut self) {
        self.stale_age_seconds = self
            .limits
            .iter()
            .map(|limit| {
                jiff::Timestamp::now().as_second().saturating_sub(limit.observed_at) as u64
            })
            .max()
            .or_else(|| self.last_live_success_at.as_deref().and_then(age_seconds));
        self.compact_status = if self.freshness == Freshness::Unavailable || self.limits.is_empty()
        {
            CompactStatus::unavailable()
        } else if self.freshness == Freshness::Stale {
            CompactStatus {
                level: CompactStatusLevel::Stale,
                label: "Data stale".to_string(),
                color: WARNING_COLOR.to_string(),
            }
        } else {
            let peak = self.limits.iter().filter_map(|limit| limit.used_percent).reduce(f64::max);
            match peak {
                Some(value) if value >= CRITICAL_PERCENT => CompactStatus {
                    level: CompactStatusLevel::Critical,
                    label: "Quota critical".to_string(),
                    color: CRITICAL_COLOR.to_string(),
                },
                Some(value) if value >= WARNING_PERCENT => CompactStatus {
                    level: CompactStatusLevel::Warning,
                    label: "Quota running low".to_string(),
                    color: WARNING_COLOR.to_string(),
                },
                Some(_) => CompactStatus {
                    level: CompactStatusLevel::Healthy,
                    label: "Quota healthy".to_string(),
                    color: HEALTHY_COLOR.to_string(),
                },
                // A window can be known without its allowance being published, which is
                // the ordinary case for a provider read from its own logs. That is a
                // working provider with less to say, not a broken one.
                None => CompactStatus {
                    level: CompactStatusLevel::Healthy,
                    label: "Window tracked".to_string(),
                    color: HEALTHY_COLOR.to_string(),
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_snapshot(used_percent: f64) -> ProviderSnapshot {
        provider_snapshot(ProviderKind::Codex, used_percent)
    }

    fn provider_snapshot(provider: ProviderKind, used_percent: f64) -> ProviderSnapshot {
        let mut snapshot = ProviderSnapshot {
            freshness: Freshness::Fresh,
            limits: vec![LimitWindow {
                kind: LimitKind::Primary,
                label: "Primary window".to_string(),
                used_percent: Some(used_percent),
                window_duration_mins: Some(300),
                resets_at: None,
                source: WindowSource::AppServer,
                observed_at: jiff::Timestamp::now().as_second(),
                freshness: Freshness::Fresh,
                status_color: String::new(),
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
        assert_eq!(fresh_snapshot(69.0).compact_status.level, CompactStatusLevel::Healthy);
        assert_eq!(fresh_snapshot(70.0).compact_status.level, CompactStatusLevel::Warning);
        assert_eq!(fresh_snapshot(90.0).compact_status.level, CompactStatusLevel::Critical);
    }

    #[test]
    fn history_success_does_not_renew_an_old_live_window() {
        let now = jiff::Timestamp::now();
        let mut snapshot = ProviderSnapshot {
            limits: vec![LimitWindow {
                kind: LimitKind::Primary,
                label: "5-hour window".to_string(),
                used_percent: Some(20.0),
                window_duration_mins: Some(300),
                resets_at: Some(now.as_second() + 3_600),
                source: WindowSource::AppServer,
                observed_at: now.as_second() - 901,
                freshness: Freshness::Fresh,
                status_color: String::new(),
            }],
            last_live_success_at: Some(now.to_string()),
            last_history_success_at: Some(now.to_string()),
            ..ProviderSnapshot::new(ProviderKind::Codex)
        };
        snapshot.resolve_derived_state();
        assert_eq!(snapshot.freshness, Freshness::Stale);
        assert_eq!(snapshot.limits[0].freshness, Freshness::Stale);
    }

    #[test]
    fn an_old_status_line_reading_is_stale_even_after_a_successful_refresh() {
        let now = jiff::Timestamp::now();
        let mut snapshot = ProviderSnapshot {
            limits: vec![LimitWindow {
                kind: LimitKind::Primary,
                label: "5-hour window".to_string(),
                used_percent: Some(20.0),
                window_duration_mins: Some(300),
                resets_at: Some(now.as_second() + 600),
                source: WindowSource::StatusLine,
                observed_at: now.as_second() - 3_601,
                freshness: Freshness::Fresh,
                status_color: String::new(),
            }],
            last_live_success_at: Some(now.to_string()),
            ..ProviderSnapshot::new(ProviderKind::Claude)
        };
        snapshot.resolve_derived_state();
        assert_eq!(snapshot.freshness, Freshness::Stale);
        assert_eq!(snapshot.limits[0].freshness, Freshness::Stale);
    }

    #[test]
    fn each_window_is_coloured_by_its_own_reading_not_the_providers() {
        let mut snapshot = provider_snapshot(ProviderKind::Claude, 92.0);
        snapshot.limits.push(LimitWindow {
            kind: LimitKind::Secondary,
            label: "Weekly window".to_string(),
            used_percent: Some(59.0),
            window_duration_mins: Some(10_080),
            resets_at: None,
            source: WindowSource::StatusLine,
            observed_at: jiff::Timestamp::now().as_second(),
            freshness: Freshness::Fresh,
            status_color: String::new(),
        });
        snapshot.last_live_success_at = Some(jiff::Timestamp::now().to_string());
        snapshot.resolve_derived_state();
        assert_eq!(snapshot.compact_status.level, CompactStatusLevel::Critical);
        assert_eq!(snapshot.limits[0].status_color, CRITICAL_COLOR);
        assert_eq!(
            snapshot.limits[1].status_color, HEALTHY_COLOR,
            "a barely used window keeps its own colour beside a spent one"
        );
    }

    #[test]
    fn stale_status_takes_priority_over_quota_thresholds() {
        let mut snapshot = fresh_snapshot(95.0);
        snapshot.freshness = Freshness::Stale;
        snapshot.update_compact_status();
        assert_eq!(snapshot.compact_status.level, CompactStatusLevel::Stale);
    }

    #[test]
    fn the_aggregate_reports_the_loudest_provider() {
        let healthy = provider_snapshot(ProviderKind::Codex, 20.0);
        let critical = provider_snapshot(ProviderKind::Codex, 95.0);
        let aggregate = aggregate_status(&[healthy.clone(), critical]);
        assert_eq!(aggregate.level, CompactStatusLevel::Critical);

        // An unreadable provider outranks one that is merely running low.
        let mut unavailable = provider_snapshot(ProviderKind::Codex, 20.0);
        unavailable.freshness = Freshness::Unavailable;
        unavailable.update_compact_status();
        let warning = provider_snapshot(ProviderKind::Codex, 80.0);
        assert_eq!(
            aggregate_status(&[warning, unavailable]).level,
            CompactStatusLevel::Unavailable
        );

        assert_eq!(aggregate_status(&[healthy]).level, CompactStatusLevel::Healthy);
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
    /// The same usage bucketed by local hour, for the recent window an hourly chart can
    /// cover. A parse reaches back as far as the provider's own sessions go, but nothing
    /// is stored hourly past [`HOURLY_HISTORY_DAYS`], so the parser stops there too.
    pub hours: Vec<HistoryHour>,
}

/// One local hour of usage, as the per-model rows it is stored and summed from.
#[derive(Debug, Clone)]
pub struct HistoryHour {
    /// The local hour this bucket opened, as `YYYY-MM-DDTHH:00`.
    pub hour_start: String,
    pub model_rows: Vec<ModelUsageRow>,
}

/// How far back hourly usage is parsed and kept. Beyond this the daily rows are the whole
/// record, which is what every range longer than a few days is drawn from anyway.
pub const HOURLY_HISTORY_DAYS: i64 = 14;

#[derive(Debug, Clone)]
pub struct HistoryDay {
    pub date: String,
    pub usage: TokenUsage,
    pub models: Vec<ModelUsage>,
    pub cost_usd: f64,
    pub model_rows: Vec<ModelUsageRow>,
}

#[derive(Debug, Clone)]
pub struct ModelUsageRow {
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
    /// This day's own model mix, largest first. The daily rows are already stored per
    /// model, so a day can say which models made it up without a second query and without
    /// the renderer reparsing anything.
    pub models: Vec<ModelUsage>,
}

/// The highest share of a quota window this machine observed on one day.
///
/// A day is summarised by its peak rather than its last reading: a window that filled and
/// restarted inside the same day is described by how full it got, and the restart itself is
/// carried separately by the reset events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaHistoryPoint {
    pub date: String,
    pub peak_used_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaHistoryWindow {
    pub kind: LimitKind,
    pub label: String,
    pub points: Vec<QuotaHistoryPoint>,
}

/// What a provider's quota did across a date range, and every restart recorded inside it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaHistorySnapshot {
    pub start_date: String,
    pub end_date: String,
    pub windows: Vec<QuotaHistoryWindow>,
    /// Restarts anchored inside the range, oldest first, so they can be drawn along the
    /// same axis as the windows above them.
    pub resets: Vec<LimitResetEvent>,
}

/// One hour of usage, the hourly counterpart of [`DailyUsagePoint`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourlyUsagePoint {
    /// The local hour this bucket opened, as `YYYY-MM-DDTHH:00`.
    pub hour_start: String,
    pub usage: TokenUsage,
    pub api_equivalent_cost_usd: Option<f64>,
    pub models: Vec<ModelUsage>,
}

/// What a range looks like at hourly resolution.
///
/// Short ranges are read hour by hour rather than day by day: three columns say nothing
/// about when a day's work happened. Only the hours with usage are returned; the renderer
/// draws the empty ones from the range itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHoursSnapshot {
    pub start_date: String,
    pub end_date: String,
    pub hours: Vec<HourlyUsagePoint>,
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
    pub app_version: String,
    pub build_commit: String,
    /// Which build this is. The same version number ships as a debug build, a portable
    /// executable and an installed one, and they behave differently enough — where the
    /// executable lives, what Claude Code's hooks point at — that a bug report about "0.1.0"
    /// is not answerable without it.
    pub build_kind: String,
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
