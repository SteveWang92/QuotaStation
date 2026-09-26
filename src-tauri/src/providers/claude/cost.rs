//! Claude Code's sessions, each priced from the pricing catalog and, where the client
//! recorded a figure of its own, beside that too.
//!
//! Claude Code writes a `cost-state` record into a session's log carrying what it charged
//! that session to. The usage parser ignores that record type, so reading it here adds a
//! second figure for a session rather than a second count of its tokens.
//!
//! Neither figure is a bill. A subscription charges nothing per token, so both sides are
//! API-equivalent estimates and the pair says whether the pinned pricing catalog still
//! agrees with Anthropic's own accounting.
//!
//! Every session the parser has entries for is listed. The client's own figure is there
//! only where the record is: Claude Code began writing it partway through its life, and a
//! session logged before that can never be filled in.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::Result;
use ccusage_adapter_claude::SessionUsageEntry;
use serde::Deserialize;

use crate::{
    domain::{SessionCost, TokenUsage},
    providers::ProviderKind,
};

/// Every session in the entries the history parse loaded, oldest first.
pub(super) fn sessions_from(entries: &[SessionUsageEntry]) -> Result<Vec<SessionCost>> {
    let reported = reported_costs(&crate::providers::usage_files(ProviderKind::Claude)?);
    Ok(sessions(computed_costs(entries), reported))
}

/// What Claude Code recorded for each session it recorded anything for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reported {
    cost_usd: f64,
    complete: bool,
    api_duration_ms: i64,
    lines_added: i64,
    lines_removed: i64,
}

/// What the pricing catalog makes of one session's entries.
#[derive(Debug, Clone, PartialEq)]
struct Computed {
    cost_usd: f64,
    independent: bool,
    usage: TokenUsage,
    /// The first and last entry of the session, which is where it is placed and how long
    /// it is said to have run. The client's own duration is not used for that: only some
    /// sessions have one, and a column that changed meaning between rows would be worse
    /// than one that measures every session the same way.
    first_entry_ms: i64,
    last_entry_ms: i64,
    /// What each model cost, so the session can name them most expensive first.
    model_costs: BTreeMap<String, f64>,
}

impl Computed {
    fn opened_at(first_entry_ms: i64) -> Self {
        Self {
            cost_usd: 0.0,
            independent: true,
            usage: TokenUsage::default(),
            first_entry_ms,
            last_entry_ms: first_entry_ms,
            model_costs: BTreeMap::new(),
        }
    }
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
    #[serde(default)]
    has_unknown_model_cost: bool,
    /// How much of the session was spent waiting on the provider, and how much code it
    /// changed. Defaulted like the flag above, so a record written before Claude Code
    /// carried one of these still reports its cost rather than dropping out of the list.
    #[serde(default, rename = "totalAPIDuration")]
    total_api_duration: i64,
    #[serde(default)]
    total_lines_added: i64,
    #[serde(default)]
    total_lines_removed: i64,
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
            complete: !record.has_unknown_model_cost,
            api_duration_ms: record.total_api_duration,
            lines_added: record.total_lines_added,
            lines_removed: record.total_lines_removed,
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
///
/// The tokens are counted here rather than read from the client's own record, so a session
/// carries the same deduplicated figures every other total in the application is built
/// from.
fn computed_costs(entries: &[SessionUsageEntry]) -> BTreeMap<String, Computed> {
    let mut computed: BTreeMap<String, Computed> = BTreeMap::new();
    for entry in entries {
        let moment = entry.timestamp.as_millis();
        let session = computed
            .entry(entry.session_id.to_string())
            .or_insert_with(|| Computed::opened_at(moment));
        session.first_entry_ms = session.first_entry_ms.min(moment);
        session.last_entry_ms = session.last_entry_ms.max(moment);
        session.cost_usd += entry.cost;
        session.independent &= !entry.has_own_cost;
        let usage = entry.usage;
        // Claude reports cache creation as its own category and no reasoning at all, which
        // is the fold the daily history already applies to the same numbers.
        let input = usage.input_tokens + usage.cache_creation_token_count();
        session.usage.input += input;
        session.usage.cache_read += usage.cache_read_input_tokens;
        session.usage.output += usage.output_tokens;
        session.usage.total += input + usage.cache_read_input_tokens + usage.output_tokens;
        if let Some(model) = &entry.model {
            let model = ccusage_core::model_aliases::resolve_model_name(model);
            *session.model_costs.entry(model.into_owned()).or_default() += entry.cost;
        }
    }
    computed
}

/// One session's models, most expensive first.
fn models_of(computed: &Computed) -> Vec<String> {
    let mut models: Vec<(&String, &f64)> = computed.model_costs.iter().collect();
    models.sort_by(|left, right| right.1.total_cmp(left.1));
    models.into_iter().map(|(model, _)| model.clone()).collect()
}

/// Every session the parser read, with the client's own figures attached where it
/// recorded them. A session the client summarised but the parser has no entries for is
/// left out rather than listed with a nought it never spent — a session can be summarised
/// in a log whose usage records were replayed into another one and deduplicated away.
fn sessions(
    computed: BTreeMap<String, Computed>,
    mut reported: BTreeMap<String, Reported>,
) -> Vec<SessionCost> {
    let mut sessions: Vec<SessionCost> = computed
        .into_iter()
        .filter_map(|(session_id, computed)| {
            let reported = reported.remove(&session_id);
            Some(SessionCost {
                session_started_at: started_at(computed.first_entry_ms)?,
                session_id,
                duration_ms: computed.last_entry_ms - computed.first_entry_ms,
                computed_cost_usd: computed.cost_usd,
                independent: computed.independent,
                reported_cost_usd: reported.map(|reported| reported.cost_usd),
                reported_complete: reported.map(|reported| reported.complete),
                api_duration_ms: reported.map(|reported| reported.api_duration_ms),
                lines_added: reported.map(|reported| reported.lines_added),
                lines_removed: reported.map(|reported| reported.lines_removed),
                usage: computed.usage.clone(),
                models: models_of(&computed),
            })
        })
        .collect();
    sessions.sort_by(|left, right| left.session_started_at.cmp(&right.session_started_at));
    sessions
}

fn started_at(milliseconds: i64) -> Option<String> {
    jiff::Timestamp::from_millisecond(milliseconds).ok().map(|start| start.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session_gap_percent;

    fn reported(cost_usd: f64) -> Reported {
        Reported { cost_usd, complete: true, api_duration_ms: 0, lines_added: 0, lines_removed: 0 }
    }

    fn computed(cost_usd: f64) -> Computed {
        Computed { cost_usd, ..Computed::opened_at(1_788_187_073_329) }
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
    fn a_session_the_parser_has_no_entries_for_is_not_listed() {
        let listed = sessions(
            BTreeMap::from([("known".to_string(), computed(2.2))]),
            BTreeMap::from([
                ("known".to_string(), reported(2.0)),
                ("gone".to_string(), reported(3.0)),
            ]),
        );
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "known");
        assert_eq!(listed[0].computed_cost_usd, 2.2);
    }

    #[test]
    fn a_session_the_client_never_priced_is_listed_without_the_clients_figures() {
        let listed = sessions(
            BTreeMap::from([
                ("priced".to_string(), computed(2.2)),
                ("unpriced".to_string(), computed(4.0)),
            ]),
            BTreeMap::from([("priced".to_string(), reported(2.0))]),
        );
        let unpriced =
            listed.iter().find(|session| session.session_id == "unpriced").expect("the session");
        assert_eq!(unpriced.computed_cost_usd, 4.0);
        assert_eq!(unpriced.reported_cost_usd, None);
        assert_eq!(unpriced.lines_added, None);
        // The unpriced session's own cost must not move a comparison it is not part of.
        assert!((session_gap_percent(&listed).expect("a gap") - 10.0).abs() < 1e-9);
    }

    #[test]
    fn the_gap_is_measured_against_what_the_client_reported() {
        let listed = sessions(
            BTreeMap::from([("a".to_string(), computed(2.2))]),
            BTreeMap::from([("a".to_string(), reported(2.0))]),
        );
        assert!((session_gap_percent(&listed).expect("a gap") - 10.0).abs() < 1e-9);
        assert_eq!(session_gap_percent(&[]), None);
    }

    /// Compares this machine's own sessions. Ignored by default because it needs a
    /// populated `~/.claude/projects`; run it with
    /// `cargo test claude_session_costs -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "requires local Claude Code session history"]
    async fn claude_session_costs_pair_with_the_computed_estimate() {
        let system_timezone = jiff::tz::TimeZone::system();
        let timezone = system_timezone.iana_name().unwrap_or("UTC");
        let (_, sessions) =
            super::super::history::read_history(timezone).await.expect("read session costs");
        println!(
            "{} session(s) read, {:.1}% apart, {} independent",
            sessions.len(),
            session_gap_percent(&sessions).unwrap_or_default(),
            sessions.iter().filter(|session| session.independent).count()
        );
        for session in sessions.iter().rev().take(5) {
            println!(
                "{}: reported {} computed ${:.4}",
                session.session_started_at,
                session.reported_cost_usd.map_or("none".to_string(), |cost| format!("${cost:.4}")),
                session.computed_cost_usd
            );
        }
    }
}
