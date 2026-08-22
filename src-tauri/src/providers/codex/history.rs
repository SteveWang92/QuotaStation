use anyhow::{Context, Result};
use std::collections::BTreeMap;

use ccusage_adapter_codex::{
    CodexGroup, CodexServiceTier, CodexSpeedPolicy, CodexTokenUsageEvent, aggregate_events,
    calculate_codex_model_cost, calculate_group_cost, load_codex_events,
};
use ccusage_core::{
    PricingMap,
    cli::{AgentReportKind, SharedArgs},
    parse_ts_timestamp,
};

use crate::{
    domain::{HistoryDay, HistoryHour, HistorySnapshot, ModelUsage, ModelUsageRow, TokenUsage},
    providers::hours,
};

pub async fn read_history(timezone: &str) -> Result<HistorySnapshot> {
    let timezone = timezone.to_string();
    tokio::task::spawn_blocking(move || read_history_blocking(&timezone))
        .await
        .context("Codex history parser stopped unexpectedly")?
}

fn read_history_blocking(timezone: &str) -> Result<HistorySnapshot> {
    let events = load_codex_events(&SharedArgs { json: true, ..SharedArgs::default() })
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let groups = aggregate_events(&events, AgentReportKind::Daily, Some(timezone))
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let pricing = PricingMap::load_embedded();
    let speed = CodexSpeedPolicy::Auto(CodexServiceTier::Standard);
    let mut days = Vec::with_capacity(groups.len());
    for (date, group) in groups {
        let cost_usd = calculate_group_cost(&group, &pricing, speed);
        let mut model_rows = model_rows_of(&group, &pricing, speed);
        model_rows.sort_by_key(|row| std::cmp::Reverse(row.total));
        let models = model_rows
            .iter()
            .map(|row| ModelUsage {
                model: row.model.clone(),
                tokens: row.total,
                percent: if group.total_tokens == 0 {
                    0.0
                } else {
                    row.total as f64 / group.total_tokens as f64 * 100.0
                },
            })
            .collect();
        days.push(HistoryDay {
            date,
            usage: TokenUsage {
                input: group.input_tokens,
                cache_read: group.cached_input_tokens,
                output: group.output_tokens,
                reasoning: group.reasoning_output_tokens,
                total: group.total_tokens,
            },
            models,
            cost_usd,
            model_rows,
        });
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(HistorySnapshot { days, hours: hourly_buckets(&events, timezone, &pricing, speed)? })
}

/// The same events grouped by the local hour they fell in.
///
/// Each bucket is aggregated by the parser's own `aggregate_events`, so an hour is counted
/// exactly the way the day containing it is: the service-tier and long-context decisions
/// that drive Codex pricing cannot be reproduced from summed totals afterwards.
fn hourly_buckets(
    events: &[CodexTokenUsageEvent],
    timezone: &str,
    pricing: &PricingMap,
    speed: CodexSpeedPolicy,
) -> Result<Vec<HistoryHour>> {
    let zone = jiff::tz::TimeZone::get(timezone).unwrap_or_else(|_| jiff::tz::TimeZone::system());
    let cutoff = hours::cutoff_date(&zone);
    let mut buckets: BTreeMap<String, Vec<CodexTokenUsageEvent>> = BTreeMap::new();
    for event in events {
        let Some(millis) = parse_ts_timestamp(&event.timestamp).map(|value| value.as_millis())
        else {
            continue;
        };
        let Some(hour_start) = hours::hour_key(millis, &zone) else { continue };
        if !hours::within_window(&hour_start, &cutoff) {
            continue;
        }
        buckets.entry(hour_start).or_default().push(event.clone());
    }

    let mut hourly = Vec::with_capacity(buckets.len());
    for (hour_start, bucket) in buckets {
        let groups = aggregate_events(&bucket, AgentReportKind::Daily, Some(timezone))
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut model_rows = Vec::new();
        for group in groups.values() {
            model_rows.extend(model_rows_of(group, pricing, speed));
        }
        model_rows.sort_by_key(|row| std::cmp::Reverse(row.total));
        hourly.push(HistoryHour { hour_start, model_rows });
    }
    Ok(hourly)
}

/// One group's per-model rows, in the shared shape both resolutions are stored in.
fn model_rows_of(
    group: &CodexGroup,
    pricing: &PricingMap,
    speed: CodexSpeedPolicy,
) -> Vec<ModelUsageRow> {
    group
        .models
        .iter()
        .map(|(model, usage)| ModelUsageRow {
            model: model.clone(),
            input: usage.input_tokens,
            cache_read: usage.cached_input_tokens,
            output: usage.output_tokens,
            reasoning: usage.reasoning_output_tokens,
            total: usage.total_tokens,
            cost_usd: calculate_codex_model_cost(model, usage, pricing, speed),
        })
        .collect()
}
