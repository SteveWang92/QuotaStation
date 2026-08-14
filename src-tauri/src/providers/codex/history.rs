use anyhow::{Context, Result};
use ccusage_adapter_codex::{
    CodexServiceTier, CodexSpeedPolicy, aggregate_events, calculate_codex_model_cost,
    calculate_group_cost, load_codex_events,
};
use ccusage_core::{PricingMap, cli::{AgentReportKind, SharedArgs}};

use crate::domain::{DailyModelUsage, HistoryDay, HistorySnapshot, ModelUsage, TokenUsage};

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
        let mut model_rows = Vec::new();
        for (model, usage) in &group.models {
            model_rows.push(DailyModelUsage {
                model: model.clone(),
                input: usage.input_tokens,
                cache_read: usage.cached_input_tokens,
                output: usage.output_tokens,
                reasoning: usage.reasoning_output_tokens,
                total: usage.total_tokens,
                cost_usd: calculate_codex_model_cost(model, usage, &pricing, speed),
            });
        }
        model_rows.sort_by(|a, b| b.total.cmp(&a.total));
        let models = model_rows.iter().map(|row| ModelUsage {
            model: row.model.clone(),
            tokens: row.total,
            percent: if group.total_tokens == 0 { 0.0 } else { row.total as f64 / group.total_tokens as f64 * 100.0 },
        }).collect();
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
    Ok(HistorySnapshot { days })
}
