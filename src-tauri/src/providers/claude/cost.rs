//! Claude Code's own cost accounting, beside the one QuotaStation computes.
//!
//! Claude Code writes a `cost-state` record into a session's log carrying what it charged
//! that session to. The usage parser ignores that record type, so reading it here adds a
//! second figure for a session rather than a second count of its tokens.
//!
//! Neither figure is a bill. A subscription charges nothing per token, so both sides are
//! API-equivalent estimates and the pair says whether the pinned pricing catalog still
//! agrees with Anthropic's own accounting.
//!
//! The comparison covers the sessions that carry the record and no others: Claude Code
//! began writing it partway through its life, and a session logged before that can never
//! be filled in.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::{Context, Result};
use ccusage_adapter_claude::load_entries;
use ccusage_core::cli::SharedArgs;
use serde::Deserialize;

use crate::{domain::SessionCost, providers::ProviderKind};

/// Every session both sides priced, oldest first.
pub async fn read_session_costs() -> Result<Vec<SessionCost>> {
    tokio::task::spawn_blocking(read_session_costs_blocking)
        .await
        .context("Claude cost reader stopped unexpectedly")?
}

fn read_session_costs_blocking() -> Result<Vec<SessionCost>> {
    let reported = reported_costs(&crate::providers::usage_files(ProviderKind::Claude)?);
    if reported.is_empty() {
        return Ok(Vec::new());
    }
    let entries = load_entries(&SharedArgs { json: true, ..SharedArgs::default() }, None)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(compare(reported, computed_costs(&entries)))
}

/// What Claude Code recorded for each session it recorded anything for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reported {
    cost_usd: f64,
    started_at_ms: i64,
    complete: bool,
}

/// What the pricing catalog makes of one session's entries.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Computed {
    cost_usd: f64,
    independent: bool,
}

/// The record as Claude Code writes it. Only the fields the comparison needs are read;
/// the durations and line counts beside them describe the session's work, not its cost.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CostState {
    #[serde(rename = "type")]
    record_type: String,
    session_id: String,
    // The only field whose name is not the camel case of its meaning.
    #[serde(rename = "totalCostUSD")]
    total_cost_usd: f64,
    start_time: i64,
    #[serde(default)]
    has_unknown_model_cost: bool,
}

fn reported_costs(files: &[std::path::PathBuf]) -> BTreeMap<String, Reported> {
    let mut reported = BTreeMap::new();
    for file in files {
        read_reported_costs(file, &mut reported);
    }
    reported
}

/// A session's record is rewritten as the session goes on, and its total only accumulates,
/// so the largest of them is the finished session.
fn read_reported_costs(file: &Path, reported: &mut BTreeMap<String, Reported>) {
    let Ok(handle) = File::open(file) else { return };
    for line in BufReader::new(handle).lines().map_while(Result::ok) {
        // The type appears in every record; the string narrows the parse to the few lines
        // that could be one of these before any JSON is built.
        if !line.contains("cost-state") {
            continue;
        }
        let Ok(record) = serde_json::from_str::<CostState>(&line) else { continue };
        if record.record_type != "cost-state" {
            continue;
        }
        let entry = Reported {
            cost_usd: record.total_cost_usd,
            started_at_ms: record.start_time,
            complete: !record.has_unknown_model_cost,
        };
        reported
            .entry(record.session_id)
            .and_modify(|kept: &mut Reported| {
                if entry.cost_usd > kept.cost_usd {
                    *kept = entry;
                }
            })
            .or_insert(entry);
    }
}

/// The same sessions as the parser prices them, and whether it reached the price on its
/// own. It prices an entry from the catalog only while the entry carries no cost of its
/// own, so one entry that does is enough to make the session's two figures one number.
fn computed_costs(entries: &[ccusage_core::LoadedEntry]) -> BTreeMap<String, Computed> {
    let mut computed: BTreeMap<String, Computed> = BTreeMap::new();
    for entry in entries {
        let session = computed
            .entry(entry.session_id.to_string())
            .or_insert(Computed { cost_usd: 0.0, independent: true });
        session.cost_usd += entry.cost;
        session.independent &= entry.data.cost_usd.is_none();
    }
    computed
}

/// The sessions both sides know about. One the parser has no entries for is left out
/// rather than compared against a nought it never spent — a session can be summarised in a
/// log whose usage records were replayed into another one and deduplicated away.
fn compare(
    reported: BTreeMap<String, Reported>,
    computed: BTreeMap<String, Computed>,
) -> Vec<SessionCost> {
    let mut sessions: Vec<SessionCost> = reported
        .into_iter()
        .filter_map(|(session_id, reported)| {
            let computed = computed.get(&session_id)?;
            Some(SessionCost {
                session_started_at: started_at(reported.started_at_ms)?,
                session_id,
                reported_cost_usd: reported.cost_usd,
                computed_cost_usd: computed.cost_usd,
                independent: computed.independent,
                reported_complete: reported.complete,
            })
        })
        .collect();
    sessions.sort_by(|left, right| left.session_started_at.cmp(&right.session_started_at));
    sessions
}

fn started_at(milliseconds: i64) -> Option<String> {
    jiff::Timestamp::from_millisecond(milliseconds).ok().map(|start| start.to_string())
}

/// How far apart the two sides of a set of sessions are, as a share of what the client
/// reported. `None` when nothing comparable was stored, which is the ordinary state of a
/// machine whose sessions all predate the record.
pub fn gap_percent(sessions: &[SessionCost]) -> Option<f64> {
    let reported: f64 = sessions.iter().map(|session| session.reported_cost_usd).sum();
    let computed: f64 = sessions.iter().map(|session| session.computed_cost_usd).sum();
    (reported > 0.0).then(|| (computed - reported) / reported * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reported(cost_usd: f64) -> Reported {
        Reported { cost_usd, started_at_ms: 1_788_187_073_329, complete: true }
    }

    #[test]
    fn the_largest_record_of_a_session_is_the_finished_one() {
        let directory = std::env::temp_dir().join("quotastation-cost-state");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create cost fixture");
        let file = directory.join("session.jsonl");
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"user","message":{"role":"user"}}"#,
                "\n",
                r#"{"type":"cost-state","sessionId":"a","totalCostUSD":1.5,"startTime":1788187073329,"hasUnknownModelCost":false}"#,
                "\n",
                r#"{"type":"cost-state","sessionId":"a","totalCostUSD":2.25,"startTime":1788187073329,"hasUnknownModelCost":true}"#,
                "\n",
                "{ not json\n",
            ),
        )
        .expect("write cost fixture");

        let mut costs = BTreeMap::new();
        read_reported_costs(&file, &mut costs);

        assert_eq!(costs.len(), 1);
        let session = costs.get("a").expect("the session");
        assert_eq!(session.cost_usd, 2.25);
        assert!(!session.complete, "the later record owns the whole row");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_session_the_parser_has_no_entries_for_is_not_compared() {
        let sessions = compare(
            BTreeMap::from([
                ("known".to_string(), reported(2.0)),
                ("gone".to_string(), reported(3.0)),
            ]),
            BTreeMap::from([("known".to_string(), Computed { cost_usd: 2.2, independent: true })]),
        );
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "known");
        assert_eq!(sessions[0].computed_cost_usd, 2.2);
    }

    #[test]
    fn the_gap_is_measured_against_what_the_client_reported() {
        let sessions = compare(
            BTreeMap::from([("a".to_string(), reported(2.0))]),
            BTreeMap::from([("a".to_string(), Computed { cost_usd: 2.2, independent: true })]),
        );
        assert!((gap_percent(&sessions).expect("a gap") - 10.0).abs() < 1e-9);
        assert_eq!(gap_percent(&[]), None);
    }

    /// Compares this machine's own sessions. Ignored by default because it needs a
    /// populated `~/.claude/projects`; run it with
    /// `cargo test claude_session_costs -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "requires local Claude Code session history"]
    async fn claude_session_costs_pair_with_the_computed_estimate() {
        let sessions = read_session_costs().await.expect("read session costs");
        println!(
            "{} session(s) compared, {:.1}% apart, {} independent",
            sessions.len(),
            gap_percent(&sessions).unwrap_or_default(),
            sessions.iter().filter(|session| session.independent).count()
        );
        for session in sessions.iter().take(5) {
            println!(
                "{}: reported ${:.4} computed ${:.4}",
                session.session_started_at, session.reported_cost_usd, session.computed_cost_usd
            );
        }
    }
}
