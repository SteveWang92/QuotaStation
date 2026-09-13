//! Codex's sessions, as the pricing catalog prices them.
//!
//! Codex records no cost of its own anywhere in its logs, so a Codex session carries one
//! figure rather than two. It is listed all the same: the question the session list
//! answers first is what a session cost and how long it ran, and only then whether the
//! client agrees about the first of those.

use std::collections::BTreeMap;

use anyhow::Result;
use ccusage_adapter_codex::{
    CodexSpeedPolicy, CodexTokenUsageEvent, aggregate_events, calculate_codex_model_cost,
    calculate_group_cost,
};
use ccusage_core::{PricingMap, cli::AgentReportKind, parse_ts_timestamp};

use crate::domain::{SessionCost, TokenUsage};

/// Every session in the events the history parse loaded, oldest first. It takes the same
/// events rather than loading its own, so a refresh reads the rollout logs once.
pub(super) fn sessions_from(
    events: Vec<CodexTokenUsageEvent>,
    pricing: &PricingMap,
    speed: CodexSpeedPolicy,
) -> Result<Vec<SessionCost>> {
    let mut by_session: BTreeMap<String, Vec<CodexTokenUsageEvent>> = BTreeMap::new();
    for event in events {
        by_session.entry(event.session_id.clone()).or_default().push(event);
    }

    let mut sessions = Vec::with_capacity(by_session.len());
    for (session_id, events) in by_session {
        if let Some(session) = session_of(session_id, &events, pricing, speed)? {
            sessions.push(session);
        }
    }
    sessions.sort_by(|left, right| left.session_started_at.cmp(&right.session_started_at));
    Ok(sessions)
}

/// The session's own identifier, out of the name the parser knows it by.
///
/// The parser names a Codex session by the rollout log it was read from, which is a dated
/// directory and a file name ending in the identifier Codex gave the session. The path is
/// no use on a surface, so the identifier is taken out of it; a name shaped differently is
/// kept as it is rather than guessed at.
fn session_id_of(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let file = file.strip_suffix(".jsonl").unwrap_or(file);
    match file.char_indices().nth_back(UUID_LENGTH - 1) {
        Some((start, _)) if is_uuid(&file[start..]) => file[start..].to_string(),
        _ => file.to_string(),
    }
}

/// The length of the identifier Codex ends a rollout file name with.
const UUID_LENGTH: usize = 36;

fn is_uuid(value: &str) -> bool {
    value.split('-').map(str::len).eq([8, 4, 4, 4, 12])
        && value.chars().all(|character| character == '-' || character.is_ascii_hexdigit())
}

/// One session summed from its own events.
///
/// The events are aggregated by the parser's own grouping rather than added up here: the
/// service-tier and long-context decisions behind Codex pricing cannot be reproduced from
/// summed totals afterwards. A session that ran across midnight comes back as one group
/// per day, and the day boundary means nothing to a session, so the groups are summed.
fn session_of(
    session_id: String,
    events: &[CodexTokenUsageEvent],
    pricing: &PricingMap,
    speed: CodexSpeedPolicy,
) -> Result<Option<SessionCost>> {
    let mut moments = events.iter().filter_map(|event| parse_ts_timestamp(&event.timestamp));
    let Some(first) = moments.next() else { return Ok(None) };
    let (first_ms, last_ms) = moments.fold((first.as_millis(), first.as_millis()), |span, at| {
        (span.0.min(at.as_millis()), span.1.max(at.as_millis()))
    });
    let Some(started_at) =
        jiff::Timestamp::from_millisecond(first_ms).ok().map(|at| at.to_string())
    else {
        return Ok(None);
    };

    let groups = aggregate_events(events, AgentReportKind::Daily, None)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut usage = TokenUsage::default();
    let mut cost_usd = 0.0;
    let mut model_costs: BTreeMap<String, f64> = BTreeMap::new();
    for group in groups.values() {
        usage.input += group.input_tokens;
        usage.cache_read += group.cached_input_tokens;
        usage.output += group.output_tokens;
        usage.reasoning += group.reasoning_output_tokens;
        usage.total += group.total_tokens;
        cost_usd += calculate_group_cost(group, pricing, speed);
        for (model, model_usage) in &group.models {
            *model_costs.entry(model.clone()).or_default() +=
                calculate_codex_model_cost(model, model_usage, pricing, speed);
        }
    }

    let mut models: Vec<(&String, &f64)> = model_costs.iter().collect();
    models.sort_by(|left, right| right.1.total_cmp(left.1));

    Ok(Some(SessionCost {
        session_id: session_id_of(&session_id),
        session_started_at: started_at,
        duration_ms: last_ms - first_ms,
        computed_cost_usd: cost_usd,
        // Nothing in a Codex log carries a price, so every figure here is the catalog's.
        independent: true,
        reported_cost_usd: None,
        reported_complete: None,
        api_duration_ms: None,
        lines_added: None,
        lines_removed: None,
        usage,
        models: models.into_iter().map(|(model, _)| model.clone()).collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_named_by_its_own_identifier_rather_than_the_log_it_was_read_from() {
        assert_eq!(
            session_id_of(
                "2026/09/01/rollout-2026-09-01T01-01-33-01a05857-0624-7f51-acee-fdd911f7a8ef"
            ),
            "01a05857-0624-7f51-acee-fdd911f7a8ef"
        );
        assert_eq!(
            session_id_of("rollout-2026-09-01T01-01-33-session.jsonl"),
            "rollout-2026-09-01T01-01-33-session"
        );
    }

    /// Reads this machine's own Codex sessions. Ignored by default because it needs a
    /// populated rollout directory; run it with
    /// `cargo test codex_sessions -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "requires local Codex session history"]
    async fn codex_sessions_are_priced_from_the_catalog_alone() {
        let (_, sessions) =
            super::super::history::read_history("UTC").await.expect("read sessions");
        println!("{} session(s) read", sessions.len());
        for session in sessions.iter().rev().take(5) {
            println!(
                "{}: {} for ${:.4} over {} tokens ({})",
                session.session_started_at,
                session.session_id,
                session.computed_cost_usd,
                session.usage.total,
                session.models.join(", ")
            );
        }
        assert!(
            sessions.iter().all(|session| session.reported_cost_usd.is_none()),
            "Codex records no cost of its own"
        );
    }
}
