use anyhow::{Context, Result};
use ccusage_adapter_claude::load_daily_summaries;
use ccusage_core::cli::SharedArgs;

use crate::domain::{DailyModelUsage, HistoryDay, HistorySnapshot, ModelUsage, TokenUsage};

pub async fn read_history() -> Result<HistorySnapshot> {
    tokio::task::spawn_blocking(read_history_blocking)
        .await
        .context("Claude history parser stopped unexpectedly")?
}

fn read_history_blocking() -> Result<HistorySnapshot> {
    // `breakdown` is what populates the per-model rows the dashboard shows.
    let summaries = load_daily_summaries(
        &SharedArgs { json: true, breakdown: true, ..SharedArgs::default() },
        None,
        false,
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let mut days = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let Some(date) = summary.date.clone() else { continue };
        let total_tokens = summary.total_tokens();

        let mut model_rows = summary
            .model_breakdowns
            .iter()
            .map(|breakdown| DailyModelUsage {
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
        model_rows.sort_by(|a, b| b.total.cmp(&a.total));

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
    Ok(HistorySnapshot { days })
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
        let history = read_history().await.expect("parse Claude history");
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
            let parts = day.usage.input + day.usage.cache_read + day.usage.output + day.usage.reasoning;
            assert_eq!(parts, day.usage.total, "categories must add up on {}", day.date);
        }
    }
}

/// Claude reports cache creation as its own category and reports no reasoning tokens,
/// while the shared model carries reasoning but not cache creation. Counting cache
/// creation as input keeps the four categories adding up to the same total the parser
/// reported, which matters more on screen than a category Codex alone can fill.
fn input_tokens(input: u64, cache_creation: u64) -> u64 {
    input + cache_creation
}
