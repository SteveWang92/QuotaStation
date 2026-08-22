use anyhow::{Context, Result};
use ccusage_adapter_claude::load_daily_and_hourly_summaries;
use ccusage_core::{UsageSummary, cli::SharedArgs};

use crate::{
    domain::{HistoryDay, HistoryHour, HistorySnapshot, ModelUsage, ModelUsageRow, TokenUsage},
    providers::hours,
};

pub async fn read_history(timezone: &str) -> Result<HistorySnapshot> {
    let timezone = timezone.to_string();
    tokio::task::spawn_blocking(move || read_history_blocking(&timezone))
        .await
        .context("Claude history parser stopped unexpectedly")?
}

fn read_history_blocking(timezone: &str) -> Result<HistorySnapshot> {
    // `breakdown` is what populates the per-model rows the dashboard shows.
    let (summaries, hourly) = load_daily_and_hourly_summaries(
        &SharedArgs {
            json: true,
            breakdown: true,
            timezone: Some(timezone.to_string()),
            ..SharedArgs::default()
        },
        None,
        false,
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let mut days = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let Some(date) = summary.date.clone() else { continue };
        let total_tokens = summary.total_tokens();

        let model_rows = model_rows_of(&summary);

        let models = model_rows
            .iter()
            .map(|row| ModelUsage {
                model: row.model.clone(),
                tokens: row.total,
                percent: if total_tokens == 0 {
                    0.0
                } else {
                    row.total as f64 / total_tokens as f64 * 100.0
                },
            })
            .collect();

        days.push(HistoryDay {
            date,
            usage: TokenUsage {
                input: input_tokens(summary.input_tokens, summary.cache_creation_tokens),
                cache_read: summary.cache_read_tokens,
                output: summary.output_tokens,
                reasoning: 0,
                total: total_tokens,
            },
            models,
            cost_usd: summary.total_cost,
            model_rows,
        });
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(HistorySnapshot { days, hours: hourly_buckets(hourly, timezone) })
}

/// The same session files, summarised by local hour instead of by local day.
///
/// The adapter answers both from one parse and one deduplication, so an hour never
/// disagrees with the day it belongs to.
fn hourly_buckets(summaries: Vec<(String, UsageSummary)>, timezone: &str) -> Vec<HistoryHour> {
    let zone = jiff::tz::TimeZone::get(timezone).unwrap_or_else(|_| jiff::tz::TimeZone::system());
    let cutoff = hours::cutoff_date(&zone);
    summaries
        .into_iter()
        .filter(|(hour_start, _)| hours::within_window(hour_start, &cutoff))
        .map(|(hour_start, summary)| HistoryHour {
            hour_start,
            model_rows: model_rows_of(&summary),
        })
        .collect()
}

/// One summary's per-model rows, largest first, in the shared shape both resolutions are
/// stored in.
fn model_rows_of(summary: &UsageSummary) -> Vec<ModelUsageRow> {
    let mut model_rows = summary
        .model_breakdowns
        .iter()
        .map(|breakdown| ModelUsageRow {
            model: breakdown.model_name.clone(),
            input: input_tokens(breakdown.input_tokens, breakdown.cache_creation_tokens),
            cache_read: breakdown.cache_read_tokens,
            output: breakdown.output_tokens,
            reasoning: 0,
            total: breakdown.input_tokens
                + breakdown.output_tokens
                + breakdown.cache_creation_tokens
                + breakdown.cache_read_tokens
                + breakdown.extra_total_tokens,
            cost_usd: breakdown.cost,
        })
        .collect::<Vec<_>>();
    model_rows.sort_by_key(|row| std::cmp::Reverse(row.total));
    model_rows
}

/// Claude reports cache creation as its own category and reports no reasoning tokens,
/// while the shared model carries reasoning but not cache creation. Counting cache
/// creation as input keeps the four categories adding up to the same total the parser
/// reported, which matters more on screen than a category Codex alone can fill.
fn input_tokens(input: u64, cache_creation: u64) -> u64 {
    input + cache_creation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_creation_is_counted_as_input() {
        assert_eq!(input_tokens(100, 25), 125);
        assert_eq!(input_tokens(100, 0), 100);
    }

    /// Parses this machine's real Claude Code transcripts. Ignored by default because it
    /// needs a populated `~/.claude/projects`; run it with
    /// `cargo test claude_history -- --ignored --nocapture` after changing this adapter.
    #[tokio::test]
    #[ignore = "requires local Claude Code session history"]
    async fn claude_history_parses_into_the_shared_daily_shape() {
        let system_timezone = jiff::tz::TimeZone::system();
        let timezone = system_timezone.iana_name().unwrap_or("UTC");
        let history = read_history(timezone).await.expect("parse Claude history");
        assert!(!history.days.is_empty(), "no Claude usage days were parsed");
        let last = history.days.last().expect("a most recent day");
        println!(
            "{} days parsed; {} total {} tokens, ${:.2}, {} models",
            history.days.len(),
            last.date,
            last.usage.total,
            last.cost_usd,
            last.models.len()
        );
        for day in &history.days {
            let parts =
                day.usage.input + day.usage.cache_read + day.usage.output + day.usage.reasoning;
            assert_eq!(parts, day.usage.total, "categories must add up on {}", day.date);
        }
    }
}
