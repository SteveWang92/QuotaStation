//! Claude quota read from Claude Code's own status line.
//!
//! Since Claude Code 2.1.80 the JSON it hands a status-line command carries
//! `rate_limits.five_hour` and `rate_limits.seven_day`, each with the percentage consumed
//! and the epoch second the window resets. That is the same pair of windows Anthropic's
//! OAuth usage endpoint reports, delivered locally: no credential is presented, nothing
//! leaves the machine, and no rate limit is shared with Claude Code's own usage display.
//!
//! Claude Code only hands that JSON to the command configured as its status line, so
//! QuotaStation registers itself as that command. The registered process does two things
//! and exits: it writes the two windows to the application data directory, and it prints a
//! status line. It never starts the interface, and a malformed or unexpected payload only
//! costs the reading — the status line still prints, because a monitor must not be able to
//! break the client it monitors.
//!
//! The windows are only as recent as the last Claude Code turn that rendered a status
//! line, which is why the session-log window stays in place as the fallback: it is derived
//! from files that are written whether or not this bridge is installed.

use std::{
    env,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::domain::{
    CRITICAL_PERCENT, Freshness, LimitKind, LimitWindow, MAX_USED_PERCENT, PaceLevel, QuotaLevel,
    WARNING_PERCENT, WindowSource, pace_level,
};
use crate::providers::ProviderKind;
use crate::settings::{
    ColourMode, ProviderLabelStyle, QuotaFormat, STATUS_LINE_ROWS, SeparatorStyle,
    StatusLineLayout, quota_segment_id,
};
use crate::summary::{QuotaSummary, QuotaWindow};

use super::{FIVE_HOUR_WINDOW_MINS, SEVEN_DAY_WINDOW_MINS, claude_home};

/// The argument that turns this executable into the status-line command. It is deliberately
/// specific: a stray argument must never be mistaken for a request to write to stdout
/// instead of opening the interface.
pub const BRIDGE_ARG: &str = "--claude-statusline";

/// Where the bridge leaves what it read, inside QuotaStation's own application data
/// directory. Several Claude Code sessions can render a status line at once, so the file is
/// replaced atomically rather than appended to.
const CACHE_FILE: &str = "claude-status-line.json";

/// A reading older than this describes windows that have long since restarted, so it is
/// treated as absent rather than as very stale data.
const MAX_READING_AGE_SECS: i64 = 14 * 24 * 60 * 60;

/// The subset of Claude Code's status-line payload QuotaStation reads. Every field is
/// optional: the payload grows between releases, and a field that is missing means the
/// window is unknown, never that the reading failed.
#[derive(Deserialize)]
struct StatusLineInput {
    rate_limits: Option<RateLimits>,
    model: Option<Model>,
    cwd: Option<String>,
    /// Which session is rendering, and what Claude Code calls it. The Stop hook is told the
    /// former and not the latter, so this is the only place the pair can be seen together.
    session_id: Option<String>,
    session_name: Option<String>,
    workspace: Option<Workspace>,
    worktree: Option<Worktree>,
    context_window: Option<ContextWindow>,
    cost: Option<Cost>,
    effort: Option<Effort>,
    thinking: Option<Thinking>,
    fast_mode: Option<bool>,
    pr: Option<PullRequest>,
    version: Option<String>,
    output_style: Option<Named>,
    vim: Option<Vim>,
    agent: Option<Named>,
    exceeds_200k_tokens: Option<bool>,
}

#[derive(Deserialize)]
struct Named {
    name: Option<String>,
}

#[derive(Deserialize)]
struct Vim {
    mode: Option<String>,
}

#[derive(Deserialize)]
struct Effort {
    level: Option<String>,
}

#[derive(Deserialize)]
struct Thinking {
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct PullRequest {
    number: Option<u64>,
    review_state: Option<String>,
}

#[derive(Deserialize)]
struct Model {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct Workspace {
    current_dir: Option<String>,
    git_worktree: Option<String>,
}

#[derive(Deserialize)]
struct Worktree {
    branch: Option<String>,
}

/// Since Claude Code 2.1.132 the token counts describe the window as it stands rather than
/// the session's running total, which is what makes them worth printing beside the share.
#[derive(Deserialize)]
struct ContextWindow {
    used_percentage: Option<f64>,
    remaining_percentage: Option<f64>,
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
    context_window_size: Option<u64>,
    current_usage: Option<CurrentUsage>,
}

/// The most recent API response's own token counts, which is the only place the cache is
/// broken out from the rest of the input.
#[derive(Deserialize)]
struct CurrentUsage {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct Cost {
    total_cost_usd: Option<f64>,
    total_duration_ms: Option<u64>,
    total_api_duration_ms: Option<u64>,
    total_lines_added: Option<u64>,
    total_lines_removed: Option<u64>,
}

#[derive(Deserialize)]
struct RateLimits {
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
struct Bucket {
    /// Percentage of the window consumed, above 100 while low-priority mode runs past the
    /// limit.
    used_percentage: Option<f64>,
    /// Epoch seconds at which the window restarts.
    resets_at: Option<i64>,
}

/// What the bridge stored, and when. The observation time is kept so a reading left behind
/// by a Claude Code session that ended days ago can be recognised as such.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
struct Reading {
    observed_at: i64,
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
}

/// The quota windows Claude Code last reported to the status line, newest reading first.
///
/// A window whose restart has already passed is dropped rather than shown as expired: the
/// session-log window covers the five-hour case, and a seven-day window that has restarted
/// says nothing about the one now running.
pub fn read_windows() -> Result<Vec<LimitWindow>> {
    let Some(reading) = load_reading()? else { return Ok(Vec::new()) };
    windows_from(&reading, crate::clock::now().as_second())
}

/// When the bridge last recorded a reading, for the interface to explain how current the
/// windows are.
pub fn last_reading_at() -> Option<i64> {
    load_reading().ok().flatten().map(|reading| reading.observed_at)
}

fn windows_from(reading: &Reading, now: i64) -> Result<Vec<LimitWindow>> {
    if now - reading.observed_at > MAX_READING_AGE_SECS {
        return Ok(Vec::new());
    }
    if reading.observed_at <= 0 || reading.observed_at > now + 300 {
        bail!("schema_incompatible: invalid status-line observation time");
    }
    let mut windows = Vec::new();
    for (bucket, kind, minutes) in [
        (reading.five_hour, LimitKind::Primary, FIVE_HOUR_WINDOW_MINS),
        (reading.seven_day, LimitKind::Secondary, SEVEN_DAY_WINDOW_MINS),
    ] {
        let Some(bucket) = bucket else { continue };
        let Some(used_percent) = bucket.used_percentage else {
            bail!("schema_incompatible: status-line bucket omitted used percentage");
        };
        let Some(resets_at) = bucket.resets_at else {
            bail!("schema_incompatible: status-line bucket omitted reset time");
        };
        if !used_percent.is_finite() || !(0.0..=MAX_USED_PERCENT).contains(&used_percent) {
            bail!("schema_incompatible: invalid status-line percentage");
        }
        if resets_at <= now {
            continue;
        }
        if resets_at > now + minutes * 60 * 2 {
            bail!("schema_incompatible: invalid status-line reset time");
        }
        windows.push(LimitWindow {
            kind,
            label: kind.window_label(Some(minutes)),
            used_percent: Some(used_percent),
            window_duration_mins: Some(minutes),
            resets_at: Some(resets_at),
            source: WindowSource::StatusLine,
            observed_at: reading.observed_at,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
            pace: PaceLevel::OnTrack,
        });
    }
    if reading.five_hour.is_some() || reading.seven_day.is_some() {
        return Ok(windows);
    }
    bail!("schema_incompatible: status-line rate limits contained no known buckets")
}

fn load_reading() -> Result<Option<Reading>> {
    let Some(path) = cache_path() else { return Ok(None) };
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read status-line cache"),
    };
    serde_json::from_str(&content)
        .map(Some)
        .context("schema_incompatible: decode status-line cache")
}

fn cache_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(CACHE_FILE))
}

/// QuotaStation's application data directory, resolved the same way Tauri resolves it, so
/// the short-lived bridge process and the running application agree on one file without the
/// bridge having to start Tauri to ask.
pub fn app_data_dir() -> Option<PathBuf> {
    const IDENTIFIER: &str = "me.stevewang.quotastation";
    #[cfg(windows)]
    {
        env::var_os("APPDATA").map(|roaming| PathBuf::from(roaming).join(IDENTIFIER))
    }
    #[cfg(not(windows))]
    {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .map(|data| data.join(IDENTIFIER))
    }
}

/// Runs as Claude Code's status-line command when this executable was started with
/// [`BRIDGE_ARG`], and reports whether it did.
///
/// Everything here is best effort. Claude Code renders whatever this prints, so a failure
/// to store the reading must still leave a status line behind.
pub fn run_bridge_if_requested() -> bool {
    if !env::args_os().any(|argument| argument == BRIDGE_ARG) {
        return false;
    }
    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);
    let input = serde_json::from_str::<StatusLineInput>(&payload).ok();
    let limits = input.as_ref().and_then(|input| input.rate_limits.as_ref());
    // This process has no console and no window, so the log is the only place it can say
    // whether Claude Code ran it and what the payload contained. Sizes and field presence
    // only: the payload also carries the session's own working context.
    crate::log::write(format!(
        "status line bridge ran: {} bytes, {}",
        payload.len(),
        match (input.is_some(), limits) {
            (false, _) => "unparsed",
            (true, None) => "no rate limits",
            // Named rather than counted, because which window is missing is the question a
            // half-empty status line raises.
            (true, Some(limits)) =>
                match (limits.five_hour.is_some(), limits.seven_day.is_some()) {
                    (true, true) => "five_hour and seven_day",
                    (true, false) => "five_hour only",
                    (false, true) => "seven_day only",
                    (false, false) => "neither window",
                },
        }
    ));
    if let Some(limits) = limits {
        let now = jiff::Timestamp::now().as_second();
        let reading =
            Reading { observed_at: now, five_hour: limits.five_hour, seven_day: limits.seven_day };
        match windows_from(&reading, now) {
            // The line above already says the reading arrived, so only a failure to keep it
            // adds anything to that.
            Ok(_) if store_reading(&reading).is_ok() => {}
            Ok(_) => crate::log::write("status line reading not stored"),
            Err(_) => {
                // Persist only the normalized quota subset, not the source payload. The
                // running app can then report schema_incompatible and preserve its LKG.
                let _ = store_reading(&reading);
                crate::log::write("status line schema incompatible");
            }
        }
    } else {
        // A payload without `rate_limits` is not a statement that the account has none: a
        // freshly started Claude Code session renders its status line before it has asked
        // the server for them, so every restart of the CLI would otherwise throw the last
        // reading away and leave Claude unavailable until the first turn. The stored
        // reading is left alone and expires on its own terms — each window is dropped once
        // its own restart time passes, and the whole reading once it is old enough.
        crate::log::write("status line reported no rate limits; the stored reading stands");
    }
    record_session(input.as_ref());
    let line = status_line(&view_of(input.as_ref(), limits));
    // The line itself, because a status line that came out empty or wrong is the whole
    // complaint and Claude Code keeps no record of what it was handed. It carries the same
    // normalized quota vocabulary as every other line here and nothing from the session.
    crate::log::write(format!("status line rendered {} char(s): {line}", line.chars().count()));
    println!("{line}");
    true
}

/// Tells the notification side what this session is called, which is the one thing the Stop
/// hook cannot find out for itself.
fn record_session(input: Option<&StatusLineInput>) {
    let Some(input) = input else { return };
    let Some(id) = input.session_id.as_deref().filter(|id| !id.is_empty()) else { return };
    let project = input
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.current_dir.as_deref())
        .or(input.cwd.as_deref())
        .and_then(|path| Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned());
    super::notifications::record_session(
        id,
        input.session_name.as_deref().filter(|name| !name.is_empty()),
        project.as_deref(),
        jiff::Timestamp::now().as_second(),
    );
}

/// Reads the payload once into the shape the renderer draws from. Every field is optional
/// in the payload and stays optional here: a missing one costs its column and nothing else.
fn view_of<'a>(
    input: Option<&'a StatusLineInput>,
    limits: Option<&RateLimits>,
) -> StatusLineView<'a> {
    let now = jiff::Timestamp::now().as_second();
    let settings = crate::settings::load_default();
    let layout = settings.status_line_layout;
    let summary = crate::summary::load_fresh(now);
    let workspace = input.and_then(|input| input.workspace.as_ref());
    let current_dir = workspace
        .and_then(|workspace| workspace.current_dir.as_deref())
        .or_else(|| input.and_then(|input| input.cwd.as_deref()))
        .map(Path::new);
    let context = input.and_then(|input| input.context_window.as_ref());
    let cost = input.and_then(|input| input.cost.as_ref());
    let repository = current_dir.and_then(crate::git::repository_root);
    let wants =
        |id: &str| layout.segments.iter().any(|segment| segment.enabled && segment.id == id);
    // Only a line that shows what a `git` process answers pays for running it.
    let git = repository
        .as_deref()
        .map(|root| {
            let wanted = crate::git::Wanted {
                status: wants("branch"),
                head: wants("lastCommit") || wants("tag"),
            };
            crate::git::read(root, now, wanted)
        })
        .unwrap_or_default();
    let operation =
        repository.as_deref().filter(|_| wants("gitOperation")).and_then(crate::git::operation);
    let stash = repository.as_deref().filter(|_| wants("stash")).and_then(crate::git::stash_count);
    let labels = settings.status_line_provider_labels;
    let today = summary.as_ref().and_then(today_totals);
    let week = summary.as_ref().and_then(week_totals);
    let last_reset = summary.as_ref().and_then(|summary| latest_reset(summary, labels));
    let reset_credits =
        summary.as_ref().map(|summary| reset_credits(summary, labels)).unwrap_or_default();
    let quota_age = summary.as_ref().map(|summary| now - summary.generated_at);
    let text =
        |value: Option<&'a String>| value.map(String::as_str).filter(|text| !text.is_empty());
    StatusLineView {
        session_name: text(input.and_then(|input| input.session_name.as_ref())),
        agent: text(input.and_then(|input| input.agent.as_ref()?.name.as_ref())),
        output_style: text(input.and_then(|input| input.output_style.as_ref()?.name.as_ref())),
        vim_mode: text(input.and_then(|input| input.vim.as_ref()?.mode.as_ref())),
        version: text(input.and_then(|input| input.version.as_ref())),
        operation,
        stash,
        head: git.head,
        large_context: input.and_then(|input| input.exceeds_200k_tokens).unwrap_or(false),
        lines_changed: cost.and_then(|cost| {
            Some((cost.total_lines_added?, cost.total_lines_removed.unwrap_or(0)))
        }),
        duration_ms: cost
            .and_then(|cost| Some((cost.total_duration_ms?, cost.total_api_duration_ms))),
        week,
        last_reset,
        reset_credits,
        quota_age,
        model: input
            .and_then(|input| input.model.as_ref())
            .and_then(|model| model.display_name.as_deref()),
        // The last segment only: the full path is this machine's business, and the status
        // line has room for the part that identifies the project.
        directory: current_dir
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned()),
        branch: input
            .and_then(|input| input.worktree.as_ref())
            .and_then(|worktree| worktree.branch.clone())
            .or_else(|| workspace.and_then(|workspace| workspace.git_worktree.clone()))
            .or_else(|| repository.as_deref().and_then(crate::git::branch_at)),
        worktree: git.status,
        context_used: context.and_then(|context| {
            context
                .used_percentage
                .or_else(|| context.remaining_percentage.map(|free| 100.0 - free))
        }),
        context_tokens: context.and_then(|context| {
            let size = context.context_window_size.filter(|size| *size > 0)?;
            let input = context.total_input_tokens?;
            Some((input + context.total_output_tokens.unwrap_or(0), size))
        }),
        cache_hit: context.and_then(|context| cache_hit(context.current_usage.as_ref()?)),
        effort: input
            .and_then(|input| input.effort.as_ref())
            .and_then(|effort| effort.level.as_deref()),
        thinking: input
            .and_then(|input| input.thinking.as_ref())
            .and_then(|thinking| thinking.enabled)
            .unwrap_or(false),
        fast_mode: input.and_then(|input| input.fast_mode).unwrap_or(false),
        pull_request: input
            .and_then(|input| input.pr.as_ref())
            .and_then(|pr| Some((pr.number?, pr.review_state.as_deref()))),
        session_cost_usd: cost.and_then(|cost| cost.total_cost_usd),
        today,
        quotas: quota_segments(windows_from_payload(limits), summary, labels),
        layout,
        now,
    }
}

/// The last seven days' tokens across every provider, and their cost where priced.
fn week_totals(summary: &QuotaSummary) -> Option<(u64, Option<f64>)> {
    let tokens: u64 = summary.providers.iter().map(|provider| provider.week_tokens).sum();
    let costs: Vec<f64> =
        summary.providers.iter().filter_map(|provider| provider.week_cost_usd).collect();
    (tokens > 0).then(|| (tokens, (!costs.is_empty()).then(|| costs.iter().sum())))
}

/// The most recent confirmed restart of any provider's window.
struct ResetNote {
    provider: String,
    window: String,
    at: i64,
    early: bool,
}

fn latest_reset(summary: &QuotaSummary, labels: ProviderLabelStyle) -> Option<ResetNote> {
    summary
        .providers
        .iter()
        .filter_map(|provider| Some((provider, provider.last_reset.as_ref()?)))
        .max_by_key(|(_, reset)| reset.at)
        .map(|(provider, reset)| ResetNote {
            provider: summary_name(provider, labels),
            window: reset.window.clone(),
            at: reset.at,
            early: reset.early,
        })
}

/// Each provider that holds reset credits, by name, with how many.
fn reset_credits(summary: &QuotaSummary, labels: ProviderLabelStyle) -> Vec<(String, u64)> {
    summary
        .providers
        .iter()
        .filter_map(|provider| {
            let count = provider.earned_reset_count.filter(|count| *count > 0)?;
            Some((summary_name(provider, labels), count))
        })
        .collect()
}

fn summary_name(provider: &crate::summary::ProviderQuota, labels: ProviderLabelStyle) -> String {
    match labels {
        ProviderLabelStyle::Short => provider.short_name.clone(),
        ProviderLabelStyle::Full => provider.display_name.clone(),
    }
}

/// How long ago something happened, in the width a status line can spare.
fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        seconds if seconds < 60 => "just now".to_string(),
        seconds if seconds < 3_600 => format!("{}m ago", seconds / 60),
        seconds if seconds < 86_400 => format!("{}h ago", seconds / 3_600),
        seconds => format!("{}d ago", seconds / 86_400),
    }
}

/// A span of milliseconds, in the same width.
fn elapsed(milliseconds: u64) -> String {
    match milliseconds / 1_000 {
        seconds if seconds < 60 => format!("{seconds}s"),
        seconds if seconds < 3_600 => format!("{}m", seconds / 60),
        seconds => format!("{}h{:02}m", seconds / 3_600, (seconds % 3_600) / 60),
    }
}

/// `v1.1.0-6-gde36776` as `v1.1.0+6`: the newest tag and the commits made since it. A
/// tagged commit describes itself as the tag alone, which is printed as it is.
fn tag_text(describe: &str) -> String {
    let mut parts = describe.rsplitn(3, '-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(hash), Some(count), Some(tag))
            if hash.starts_with('g') && count.parse::<u32>().is_ok() =>
        {
            format!("{tag}+{count}")
        }
        _ => describe.to_string(),
    }
}

/// Today's tokens across every provider, and their cost where any provider priced them.
fn today_totals(summary: &QuotaSummary) -> Option<(u64, Option<f64>)> {
    let tokens: u64 = summary.providers.iter().map(|provider| provider.today_tokens).sum();
    let costs: Vec<f64> =
        summary.providers.iter().filter_map(|provider| provider.api_equivalent_cost_usd).collect();
    (tokens > 0).then(|| (tokens, (!costs.is_empty()).then(|| costs.iter().sum())))
}

/// Everything the status line is drawn from, gathered before anything is rendered so the
/// rendering itself touches neither the payload nor the disk.
struct StatusLineView<'a> {
    model: Option<&'a str>,
    directory: Option<String>,
    branch: Option<String>,
    /// What the checkout carries beyond the branch name: uncommitted paths, and the
    /// distance from the upstream.
    worktree: Option<crate::git::WorkTreeStatus>,
    context_used: Option<f64>,
    /// Tokens in the window and the window's size, so the share has a magnitude beside it.
    context_tokens: Option<(u64, u64)>,
    /// The share of the last response's input that was served from cache.
    cache_hit: Option<f64>,
    effort: Option<&'a str>,
    thinking: bool,
    fast_mode: bool,
    /// The open pull request's number and, when it has one, its review state.
    pull_request: Option<(u64, Option<&'a str>)>,
    session_cost_usd: Option<f64>,
    session_name: Option<&'a str>,
    agent: Option<&'a str>,
    output_style: Option<&'a str>,
    vim_mode: Option<&'a str>,
    version: Option<&'a str>,
    /// A rebase, merge, cherry-pick, revert or bisect left in progress.
    operation: Option<String>,
    stash: Option<usize>,
    head: Option<crate::git::HeadCommit>,
    large_context: bool,
    /// Lines this session added and removed.
    lines_changed: Option<(u64, u64)>,
    /// The session's wall-clock time and the part of it spent on the API, in milliseconds.
    duration_ms: Option<(u64, Option<u64>)>,
    /// The last seven days' tokens across every provider, and their cost when known.
    week: Option<(u64, Option<f64>)>,
    last_reset: Option<ResetNote>,
    reset_credits: Vec<(String, u64)>,
    /// How old the summary behind the other providers' quota is, in seconds.
    quota_age: Option<i64>,
    /// Today's tokens across every provider, and their API-equivalent cost when known.
    today: Option<(u64, Option<f64>)>,
    /// One entry per provider that has something to report.
    quotas: Vec<QuotaSegment>,
    layout: StatusLineLayout,
    now: i64,
}

struct QuotaSegment {
    /// The layout's name for this provider's segment.
    id: String,
    label: String,
    /// Whether this is the client the line is being rendered inside. Only its own quota can
    /// go unnamed, because only its own quota is what an unnamed reading would be read as.
    own: bool,
    windows: Vec<QuotaWindow>,
}

const GREEN: &str = "\u{1b}[32m";
const RED: &str = "\u{1b}[31m";
const YELLOW: &str = "\u{1b}[33m";
const RESET: &str = "\u{1b}[0m";

/// Text describing a share consumed, in that share's colour when colour is wanted.
///
/// The thresholds are the ones the interface uses for its warning and critical statuses, so
/// the status line and the application never disagree about when a window has become worth
/// acting on. Everything below them is green rather than left plain: a row carrying four
/// readings is scanned rather than read, and an uncoloured reading is one the eye has to
/// stop and parse before it can rule it out.
fn coloured(used: f64, text: &str, colour: bool) -> String {
    if !colour {
        return text.to_string();
    }
    let colour = match used.clamp(0.0, 100.0) {
        value if value >= CRITICAL_PERCENT => RED,
        value if value >= WARNING_PERCENT => YELLOW,
        _ => GREEN,
    };
    format!("{colour}{text}{RESET}")
}

/// Five cells, each a fifth of the window consumed.
fn bar(used: f64) -> String {
    let filled = (used.clamp(0.0, 100.0) / 20.0).round() as usize;
    format!("{}{}", "\u{25b0}".repeat(filled), "\u{25b1}".repeat(5 - filled))
}

/// One quota window, printed with the parts the layout asks for. The pace marker sits
/// inside the colour, so the arrow reads as part of the number rather than as a symbol
/// standing between two columns.
fn window_text(window: &QuotaWindow, format: QuotaFormat, colour: bool, now: i64) -> String {
    let used = window.used_percent.clamp(0.0, MAX_USED_PERCENT);
    let mut values = Vec::new();
    if format.used {
        values.push(format!("{used:.0}%"));
    }
    if format.remaining {
        values.push(format!("{:.0}% left", (100.0 - used).max(0.0)));
    }
    let marker = format.pace.then(|| pace_marker(window, now)).flatten();
    if let (Some(first), Some(marker)) = (values.first_mut(), marker) {
        first.push_str(marker);
    }
    if format.bar {
        values.push(bar(used));
    }
    let mut parts = vec![window.label.clone()];
    if !values.is_empty() {
        parts.push(coloured(used, &values.join(" "), colour));
    }
    if format.countdown
        && let Some(left) = window.resets_at.and_then(|resets_at| countdown(resets_at, now))
    {
        parts.push(format!("({left})"));
    }
    parts.join(" ")
}

/// How much of the last response's input came from cache rather than being sent again.
///
/// Cache reads are the cheap part of an expensive turn, so a session whose share has
/// collapsed has just paid to rebuild a prefix — worth knowing while it is happening rather
/// than in the bill afterwards.
fn cache_hit(usage: &CurrentUsage) -> Option<f64> {
    let read = usage.cache_read_input_tokens.unwrap_or(0);
    let total =
        read + usage.cache_creation_input_tokens.unwrap_or(0) + usage.input_tokens.unwrap_or(0);
    (total > 0).then(|| read as f64 / total as f64 * 100.0)
}

/// The one character a pace worth marking is written as. The rule behind it is the core's,
/// so the arrow here and the marker in the panel are never drawn from two comparisons.
fn pace_marker(window: &QuotaWindow, now: i64) -> Option<&'static str> {
    match pace_level(Some(window.used_percent), window.resets_at, window.window_minutes, now) {
        PaceLevel::Ahead => Some("\u{2191}"),
        PaceLevel::Behind => Some("\u{2193}"),
        PaceLevel::OnTrack => None,
    }
}

/// A token count in the width a status line can spare, written the way `/context` writes it
/// so the two can be compared without arithmetic.
fn tokens_short(tokens: u64) -> String {
    match tokens {
        tokens if tokens >= 1_000_000 => {
            let millions = tokens as f64 / 1_000_000.0;
            if millions >= 10.0 { format!("{millions:.0}M") } else { format!("{millions:.1}M") }
        }
        tokens if tokens >= 1_000 => format!("{:.1}k", tokens as f64 / 1_000.0),
        tokens => tokens.to_string(),
    }
}

/// How long a window has left, in the width a status line can spare. Whole days carry the
/// point on their own; below a day the minutes are what decide whether to keep working.
fn countdown(resets_at: i64, now: i64) -> Option<String> {
    let minutes = (resets_at - now) / 60;
    if minutes <= 0 {
        return None;
    }
    let (days, hours, minutes) = (minutes / 1_440, (minutes % 1_440) / 60, minutes % 60);
    Some(match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, hours) => format!("{hours}h{minutes:02}m"),
        (days, _) => format!("{days}d"),
    })
}

/// What the checkout carries, in the shorthand `git` users already read: `*` for paths that
/// differ from the last commit, an arrow for each commit the branch stands away from its
/// upstream. A clean branch that is level with its remote prints nothing at all, which is
/// what makes the marks worth a glance when they do appear.
fn work_tree_marks(status: crate::git::WorkTreeStatus) -> String {
    let mut marks = Vec::new();
    if status.changed > 0 {
        marks.push(format!("*{}", status.changed));
    }
    if status.ahead > 0 {
        marks.push(format!("\u{2191}{}", status.ahead));
    }
    if status.behind > 0 {
        marks.push(format!("\u{2193}{}", status.behind));
    }
    marks.join(" ")
}

/// The mark inside a group of readings of one kind, and the one between groups. The eye
/// stops at the second, which is what tells the model apart from the project, the project
/// from the session's own cost, and one provider from the next.
fn separators(style: SeparatorStyle) -> (&'static str, &'static str) {
    match style {
        SeparatorStyle::Classic => (" \u{b7} ", " | "),
        SeparatorStyle::Arrow => (" \u{b7} ", " \u{203a} "),
        SeparatorStyle::Powerline => (" \u{b7} ", " \u{e0b1} "),
    }
}

/// Which readings belong together. Neighbouring segments of one group take the lighter
/// mark, which is what lets any order still read as groups rather than as a flat list.
fn group(id: &str) -> &str {
    match id {
        "model" | "mode" | "agent" | "outputStyle" | "vimMode" => "model",
        "directory" | "branch" | "gitOperation" | "stash" | "lastCommit" | "tag"
        | "pullRequest" => "project",
        "context" | "largeContext" | "cache" => "request",
        "linesChanged" | "duration" => "work",
        "today" | "week" => "usage",
        "lastReset" | "resetCredits" => "resets",
        // The session's name and cost, the client's version, how old the quota is, and each
        // provider's quota stand alone.
        other => other,
    }
}

/// What Claude Code shows while the bridge is installed.
///
/// The layout decides which segments appear, on which of up to three rows, and in what
/// order. The default puts what this session is on the first row, what it has consumed on
/// the second, and what is left across every provider QuotaStation watches on the third.
/// That last one is the whole reason the bridge is worth its screen — Claude Code knows its
/// own quota and nothing about anyone else's, and this is the one place both can be read
/// without leaving the terminal.
fn status_line(view: &StatusLineView) -> String {
    let layout = &view.layout;
    let (within, between) = separators(layout.separators);
    let quota_colour = layout.colour != ColourMode::None;
    let enabled =
        |id: &str| layout.segments.iter().any(|segment| segment.enabled && segment.id == id);

    let quotas: Vec<(&QuotaSegment, String)> = view
        .quotas
        .iter()
        .filter(|segment| enabled(&segment.id))
        .filter_map(|segment| {
            let windows: Vec<String> = segment
                .windows
                .iter()
                // A window that has already restarted describes nothing that is running now.
                .filter(|window| window.resets_at.is_none_or(|resets_at| resets_at > view.now))
                .map(|window| window_text(window, layout.quota, quota_colour, view.now))
                .collect();
            (!windows.is_empty()).then(|| (segment, windows.join(within)))
        })
        .collect();
    // Naming the provider only matters once there is a second one to tell it apart from;
    // alone inside Claude Code it says what the line is already running in. A foreign
    // provider is always named, whatever else survived: an unnamed reading beside Claude
    // Code's own model is read as Claude Code's own quota.
    let several = quotas.len() > 1;

    let text = |id: &str| -> Option<String> {
        match id {
            "model" => view.model.filter(|model| !model.is_empty()).map(str::to_string),
            "mode" => {
                let mut mode = Vec::new();
                if let Some(effort) = view.effort.filter(|level| !level.is_empty()) {
                    mode.push(effort);
                }
                if view.fast_mode {
                    mode.push("fast");
                }
                if view.thinking {
                    mode.push("think");
                }
                (!mode.is_empty()).then(|| mode.join(within))
            }
            "directory" => view.directory.clone(),
            "branch" => view.branch.as_ref().map(|branch| {
                match view.worktree.map(work_tree_marks).filter(|marks| !marks.is_empty()) {
                    Some(marks) => format!("{branch} {marks}"),
                    None => branch.clone(),
                }
            }),
            "pullRequest" => view.pull_request.map(|(number, state)| {
                match state.filter(|state| !state.is_empty()) {
                    Some(state) => format!("PR #{number} {state}"),
                    None => format!("PR #{number}"),
                }
            }),
            "context" => view.context_used.map(|used| {
                let share = coloured(
                    used,
                    &format!("{:.0}%", used.clamp(0.0, 100.0)),
                    layout.colour == ColourMode::Full,
                );
                match view.context_tokens {
                    Some((tokens, size)) => {
                        format!("ctx {share} ({}/{})", tokens_short(tokens), tokens_short(size))
                    }
                    None => format!("ctx {share}"),
                }
            }),
            // One decimal, because in Claude Code almost every input token is a cache read
            // and the whole figure rounds to 100% on an ordinary turn. The turns worth
            // noticing are the ones that had to rebuild part of the prefix, and that shows
            // in the fraction.
            "cache" => view.cache_hit.map(|hit| format!("cache {hit:.1}%")),
            // A session that has spent nothing yet has no figure worth a column.
            "sessionCost" => {
                view.session_cost_usd.filter(|cost| *cost > 0.0).map(|cost| format!("${cost:.2}"))
            }
            "sessionName" => view.session_name.map(str::to_string),
            "agent" => view.agent.map(|name| format!("agent {name}")),
            // Every session has the default style, so only a chosen one says anything.
            "outputStyle" => view
                .output_style
                .filter(|style| *style != "default")
                .map(|style| format!("style {style}")),
            "vimMode" => view.vim_mode.map(str::to_string),
            "version" => view.version.map(|version| format!("v{version}")),
            "gitOperation" => view.operation.clone(),
            "stash" => view.stash.filter(|count| *count > 0).map(|count| format!("stash {count}")),
            "lastCommit" => view
                .head
                .as_ref()
                .map(|head| format!("commit {}", ago(view.now - head.committed_at))),
            "tag" => view.head.as_ref().and_then(|head| head.describe.as_deref()).map(tag_text),
            "largeContext" => view.large_context.then(|| {
                if layout.colour == ColourMode::Full {
                    format!("{RED}ctx >200k{RESET}")
                } else {
                    "ctx >200k".to_string()
                }
            }),
            "linesChanged" => {
                view.lines_changed.map(|(added, removed)| format!("+{added} \u{2212}{removed}"))
            }
            "duration" => view.duration_ms.map(|(total, api)| match api {
                Some(api) => format!("{} (api {})", elapsed(total), elapsed(api)),
                None => elapsed(total),
            }),
            "week" => view.week.map(|(tokens, cost)| match cost {
                Some(cost) => format!("week {} ${cost:.2}", tokens_short(tokens)),
                None => format!("week {}", tokens_short(tokens)),
            }),
            "lastReset" => view.last_reset.as_ref().map(|reset| {
                format!(
                    "{} {} reset{} {}",
                    reset.provider,
                    reset.window,
                    if reset.early { " early" } else { "" },
                    ago(view.now - reset.at)
                )
            }),
            "resetCredits" => (!view.reset_credits.is_empty()).then(|| {
                view.reset_credits
                    .iter()
                    .map(|(provider, count)| format!("{provider} credits {count}"))
                    .collect::<Vec<_>>()
                    .join(within)
            }),
            "quotaAge" => view.quota_age.map(|age| format!("quota read {}", ago(age))),
            "today" => view.today.map(|(tokens, cost)| match cost {
                Some(cost) => format!("today {} ${cost:.2}", tokens_short(tokens)),
                None => format!("today {}", tokens_short(tokens)),
            }),
            quota => {
                quotas.iter().find(|(segment, _)| segment.id == quota).map(|(segment, windows)| {
                    if several || !segment.own {
                        format!("{} {windows}", segment.label)
                    } else {
                        windows.clone()
                    }
                })
            }
        }
    };

    let mut lines = Vec::new();
    for row in 1..=STATUS_LINE_ROWS {
        let mut line = String::new();
        let mut previous: Option<&str> = None;
        for segment in
            layout.segments.iter().filter(|segment| segment.enabled && segment.row == row)
        {
            let Some(text) = text(&segment.id) else { continue };
            if let Some(previous) = previous {
                line.push_str(if previous == group(&segment.id) { within } else { between });
            }
            line.push_str(&text);
            previous = Some(group(&segment.id));
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        // Claude Code hands out no rate limits on API-key and enterprise sign-ins, and an
        // empty line would read as a broken status line rather than an absent quota.
        return "QuotaStation".to_string();
    }
    lines.join("\n")
}

/// The quota windows this very payload reported, which are always newer than anything on
/// disk: they describe the turn being rendered.
fn windows_from_payload(limits: Option<&RateLimits>) -> Vec<QuotaWindow> {
    let Some(limits) = limits else { return Vec::new() };
    [
        (limits.five_hour, "5h", FIVE_HOUR_WINDOW_MINS),
        (limits.seven_day, "7d", SEVEN_DAY_WINDOW_MINS),
    ]
    .into_iter()
    .filter_map(|(bucket, label, minutes)| {
        let bucket = bucket?;
        Some(QuotaWindow {
            label: label.to_string(),
            used_percent: bucket.used_percentage?.clamp(0.0, MAX_USED_PERCENT),
            resets_at: bucket.resets_at,
            window_minutes: Some(minutes),
        })
    })
    .collect()
}

/// Every provider's quota, this client's first.
///
/// Claude's own windows come from the payload when it carried them, because those describe
/// this turn while the summary describes the last refresh. Everything else can only come
/// from the summary, and an absent or stale summary simply leaves those providers out —
/// the row says less rather than saying something no longer true.
fn quota_segments(
    payload_windows: Vec<QuotaWindow>,
    summary: Option<QuotaSummary>,
    labels: ProviderLabelStyle,
) -> Vec<QuotaSegment> {
    let name = |provider: &crate::summary::ProviderQuota| summary_name(provider, labels);
    let own_id = quota_segment_id(ProviderKind::Claude);
    let mut recorded = summary.map(|summary| summary.providers).unwrap_or_default();
    let claude =
        recorded.iter().position(|provider| provider.provider == ProviderKind::Claude.key());
    let mut segments = Vec::new();
    match (payload_windows.is_empty(), claude) {
        (false, index) => {
            let label = index
                .map(|index| name(&recorded.remove(index)))
                // The application has never recorded this provider, so the name has to come
                // from the one place that always knows it.
                .unwrap_or_else(|| provider_label(ProviderKind::Claude, labels));
            segments.push(QuotaSegment { id: own_id, label, own: true, windows: payload_windows });
        }
        (true, Some(index)) => {
            let provider = recorded.remove(index);
            segments.push(QuotaSegment {
                id: own_id,
                label: name(&provider),
                own: true,
                windows: provider.windows,
            });
        }
        (true, None) => {}
    }
    segments.extend(recorded.into_iter().map(|provider| QuotaSegment {
        id: format!("quota:{}", provider.provider),
        label: name(&provider),
        own: false,
        windows: provider.windows,
    }));
    segments
}

fn provider_label(kind: ProviderKind, labels: ProviderLabelStyle) -> String {
    match labels {
        ProviderLabelStyle::Short => kind.short_name().to_string(),
        ProviderLabelStyle::Full => kind.display_name().to_string(),
    }
}

/// The line a layout draws, from a fixed sample session rather than a real one, so the
/// settings page shows the effect of a choice through this renderer rather than a second
/// one of its own.
pub fn preview(layout: StatusLineLayout, labels: ProviderLabelStyle) -> String {
    let now = jiff::Timestamp::now().as_second();
    let window = |label: &str, used: f64, minutes_left: i64, window_minutes: i64| QuotaWindow {
        label: label.to_string(),
        used_percent: used,
        resets_at: Some(now + minutes_left * 60),
        window_minutes: Some(window_minutes),
    };
    let samples = ProviderKind::ALL.into_iter().map(|kind| QuotaSegment {
        id: quota_segment_id(kind),
        label: provider_label(kind, labels),
        own: kind == ProviderKind::Claude,
        // Readings chosen to show every colour and both pace markers.
        windows: match kind {
            ProviderKind::Claude => vec![
                window("5h", 42.0, 150, FIVE_HOUR_WINDOW_MINS),
                window("7d", 18.0, 5 * 1_440, SEVEN_DAY_WINDOW_MINS),
            ],
            ProviderKind::Codex => vec![
                window("5h", 76.0, 200, FIVE_HOUR_WINDOW_MINS),
                window("7d", 93.0, 1_440, SEVEN_DAY_WINDOW_MINS),
            ],
        },
    });
    status_line(&StatusLineView {
        model: Some("Opus"),
        directory: Some("my-project".to_string()),
        branch: Some("main".to_string()),
        worktree: Some(crate::git::WorkTreeStatus {
            changed: 2,
            ahead: 1,
            behind: 0,
            tracked: true,
        }),
        context_used: Some(34.0),
        context_tokens: Some((340_000, 1_000_000)),
        cache_hit: Some(96.4),
        effort: Some("high"),
        thinking: true,
        fast_mode: false,
        pull_request: Some((42, Some("approved"))),
        session_cost_usd: Some(1.84),
        session_name: Some("refactor-auth"),
        agent: Some("reviewer"),
        output_style: Some("explanatory"),
        vim_mode: Some("NORMAL"),
        version: Some("2.1.140"),
        operation: Some("REBASE 2/5".to_string()),
        stash: Some(1),
        head: Some(crate::git::HeadCommit {
            committed_at: now - 3 * 3_600,
            describe: Some("v1.4.0-6-g1a2b3c4".to_string()),
        }),
        large_context: true,
        lines_changed: Some((156, 23)),
        duration_ms: Some((45 * 60_000, Some(18 * 60_000))),
        week: Some((85_000_000, Some(120.5))),
        last_reset: Some(ResetNote {
            provider: provider_label(ProviderKind::Codex, labels),
            window: "5h".to_string(),
            at: now - 2 * 3_600,
            early: true,
        }),
        reset_credits: vec![(provider_label(ProviderKind::Codex, labels), 2)],
        quota_age: Some(240),
        today: Some((12_400_000, Some(18.25))),
        quotas: samples.collect(),
        layout: layout.normalized(),
        now,
    })
}

fn store_reading(reading: &Reading) -> Result<()> {
    let path = cache_path().context("resolve the application data directory")?;
    // Concurrent Claude Code sessions render status lines independently, so a reader must
    // never see a half-written reading.
    crate::fs_atomic::write(&path, serde_json::to_string(reading)?)
        .context("store the status-line reading")
}

/// Whether Claude Code is configured to hand its quota to QuotaStation, and what stands in
/// the way when it is not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub installed: bool,
    /// A status line belonging to something else. Replacing it is the user's call, so it is
    /// reported rather than overwritten.
    pub has_foreign_command: bool,
    pub last_reading_at: Option<i64>,
    /// Claude Code is running, but only in hosts that render their own interface instead of
    /// a status line. Nothing the bridge does can produce a reading from those sessions, so
    /// an installation that looks inert has to be able to say that is why.
    pub desktop_only_sessions: bool,
}

/// Whether the status line Claude Code runs is QuotaStation's own.
///
/// [`bridge_status`] answers this too, but it also scans the session logs for the
/// interface's sake; the uninstall path wants the one fact and none of that work.
pub fn installed() -> bool {
    configured_command().as_deref().is_some_and(is_bridge_command)
}

pub fn bridge_status() -> BridgeStatus {
    let configured = configured_command();
    let ours = configured.as_deref().is_some_and(is_bridge_command);
    BridgeStatus {
        installed: ours,
        has_foreign_command: configured.is_some() && !ours,
        last_reading_at: last_reading_at(),
        desktop_only_sessions: super::sessions::live_sessions().desktop_only(),
    }
}

pub(super) fn settings_path() -> Result<PathBuf> {
    Ok(claude_home()
        .context("locate the Claude Code configuration directory")?
        .join("settings.json"))
}

fn load_settings(path: &Path) -> Result<serde_json::Value> {
    Ok(load_settings_with_source(path)?.0)
}

pub(super) fn load_settings_with_source(
    path: &Path,
) -> Result<(serde_json::Value, Option<Vec<u8>>)> {
    let source = match std::fs::read(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((serde_json::Value::Object(serde_json::Map::new()), None));
        }
        Err(error) => return Err(error).context("read the Claude Code settings"),
    };
    let content = std::str::from_utf8(&source).context("decode the Claude Code settings")?;
    if content.trim().is_empty() {
        return Ok((serde_json::Value::Object(serde_json::Map::new()), Some(source)));
    }
    let settings = serde_json::from_str(content).context("parse the Claude Code settings")?;
    Ok((settings, Some(source)))
}

fn configured_command() -> Option<String> {
    command_in(&load_settings(&settings_path().ok()?).ok()?)
}

fn command_in(settings: &serde_json::Value) -> Option<String> {
    settings.get("statusLine")?.get("command")?.as_str().map(str::to_string)
}

fn is_bridge_command(command: &str) -> bool {
    command.contains(BRIDGE_ARG)
}

/// The command Claude Code has to run. The executable is quoted because it lives under a
/// path with spaces on an ordinary Windows installation.
pub fn bridge_command() -> Result<String> {
    let executable = env::current_exe().context("locate the QuotaStation executable")?;
    Ok(format!("\"{}\" {BRIDGE_ARG}", executable.display()))
}

/// Registers QuotaStation as Claude Code's status-line command, leaving every other setting
/// as it was. A status line belonging to something else is never replaced.
pub fn install() -> Result<()> {
    install_into(&settings_path()?, &bridge_command()?)
}

/// Removes the status line again, and only ever the one QuotaStation installed.
pub fn remove() -> Result<()> {
    remove_from(&settings_path()?)?;
    if let Some(cache) = cache_path() {
        let _ = std::fs::remove_file(cache);
    }
    Ok(())
}

fn install_into(path: &Path, command: &str) -> Result<()> {
    let (mut settings, source) = load_settings_with_source(path)?;
    if let Some(existing) = command_in(&settings)
        && !is_bridge_command(&existing)
    {
        bail!(
            "Claude Code already runs its own status line. Remove that status line first, \
             then install this one."
        );
    }
    let object =
        settings.as_object_mut().context("the Claude Code settings are not a JSON object")?;
    object.insert(
        "statusLine".to_string(),
        serde_json::json!({ "type": "command", "command": command, "padding": 0 }),
    );
    write_settings(path, &settings, source.as_deref())
}

fn remove_from(path: &Path) -> Result<()> {
    let (mut settings, source) = load_settings_with_source(path)?;
    if !command_in(&settings).is_some_and(|command| is_bridge_command(&command)) {
        return Ok(());
    }
    if let Some(object) = settings.as_object_mut() {
        object.remove("statusLine");
    }
    write_settings(path, &settings, source.as_deref())
}

pub(super) fn write_settings(
    path: &Path,
    settings: &serde_json::Value,
    original: Option<&[u8]>,
) -> Result<()> {
    let current = match std::fs::read(path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("re-read the Claude Code settings"),
    };
    anyhow::ensure!(
        current.as_deref() == original,
        "Claude Code settings changed while the status line was being updated; try again"
    );
    let content = serde_json::to_string_pretty(settings)?;
    // Claude Code reads this file continuously, so it is replaced whole rather than
    // truncated and rewritten in place.
    crate::fs_atomic::write(path, format!("{content}\n")).context("write the Claude Code settings")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SegmentPlacement;

    fn reading(now: i64) -> Reading {
        Reading {
            observed_at: now,
            five_hour: Some(Bucket { used_percentage: Some(23.5), resets_at: Some(now + 3_600) }),
            seven_day: Some(Bucket { used_percentage: Some(41.2), resets_at: Some(now + 86_400) }),
        }
    }

    #[test]
    fn both_reported_windows_map_onto_the_shared_quota_vocabulary() {
        let now = 1_800_000_000;
        let windows = windows_from(&reading(now), now).expect("valid reading");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "5-hour window");
        assert_eq!(windows[0].resets_at, Some(now + 3_600));
        assert_eq!(windows[1].label, "Weekly window");
        assert_eq!(windows[1].kind, LimitKind::Secondary);
    }

    #[test]
    fn a_window_that_has_already_restarted_is_dropped_rather_than_shown_expired() {
        let now = 1_800_000_000;
        let mut stale = reading(now);
        stale.five_hour = Some(Bucket { used_percentage: Some(90.0), resets_at: Some(now - 60) });
        let windows = windows_from(&stale, now).expect("remaining window is valid");
        assert_eq!(windows.len(), 1, "only the weekly window is still running");
        assert_eq!(windows[0].kind, LimitKind::Secondary);
    }

    #[test]
    fn a_reading_left_by_a_long_finished_session_is_treated_as_absent() {
        let now = 1_800_000_000;
        let old = Reading { observed_at: now - MAX_READING_AGE_SECS - 1, ..reading(now) };
        assert!(windows_from(&old, now).expect("old reading is ignored").is_empty());
    }

    #[test]
    fn usage_past_the_limit_in_low_priority_mode_is_kept_as_reported() {
        let now = 1_800_000_000;
        let mut over = reading(now);
        over.five_hour =
            Some(Bucket { used_percentage: Some(101.0), resets_at: Some(now + 3_600) });
        let windows = windows_from(&over, now).expect("an over-limit reading is valid");
        assert_eq!(windows[0].used_percent, Some(101.0));
    }

    #[test]
    fn malformed_known_bucket_is_schema_incompatible() {
        let now = 1_800_000_000;
        let malformed = Reading {
            observed_at: now,
            five_hour: Some(Bucket { used_percentage: None, resets_at: Some(now + 3_600) }),
            seven_day: None,
        };
        let error = windows_from(&malformed, now).expect_err("missing percentage must fail");
        assert!(error.to_string().contains("schema_incompatible"));
    }

    const NOW: i64 = 1_800_000_000;

    fn view<'a>(model: Option<&'a str>, quotas: Vec<QuotaSegment>) -> StatusLineView<'a> {
        StatusLineView {
            model,
            directory: None,
            branch: None,
            worktree: None,
            context_used: None,
            context_tokens: None,
            cache_hit: None,
            effort: None,
            thinking: false,
            fast_mode: false,
            pull_request: None,
            session_cost_usd: None,
            session_name: None,
            agent: None,
            output_style: None,
            vim_mode: None,
            version: None,
            operation: None,
            stash: None,
            head: None,
            large_context: false,
            lines_changed: None,
            duration_ms: None,
            week: None,
            last_reset: None,
            reset_credits: Vec::new(),
            quota_age: None,
            today: None,
            quotas,
            layout: StatusLineLayout::default(),
            now: NOW,
        }
    }

    fn window(label: &str, used: f64, resets_in: Option<i64>) -> QuotaWindow {
        QuotaWindow {
            label: label.to_string(),
            used_percent: used,
            resets_at: resets_in.map(|seconds| NOW + seconds),
            window_minutes: None,
        }
    }

    /// Percentages without their colour, which is what the assertions are about.
    fn plain(line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(start) = rest.find('\u{1b}') {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            match rest.find('m') {
                Some(end) => rest = &rest[end + 1..],
                None => break,
            }
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn a_payload_without_rate_limits_still_produces_a_status_line() {
        assert_eq!(status_line(&view(None, Vec::new())), "QuotaStation");
        assert_eq!(status_line(&view(Some("Opus"), Vec::new())), "Opus");
    }

    fn two_providers() -> Vec<QuotaSegment> {
        vec![
            QuotaSegment {
                id: "quota:claude".to_string(),
                label: "CLD".to_string(),
                own: true,
                windows: vec![
                    window("5h", 23.5, Some(4 * 3_600 + 120)),
                    window("7d", 41.0, Some(5 * 86_400)),
                ],
            },
            QuotaSegment {
                id: "quota:codex".to_string(),
                label: "CDX".to_string(),
                own: false,
                windows: vec![window("5h", 62.0, Some(2 * 3_600 + 600))],
            },
        ]
    }

    #[test]
    fn each_row_answers_one_question_and_a_bar_separates_the_kinds_inside_it() {
        let mut session = view(Some("Opus"), two_providers());
        session.directory = Some("QuotaStation".to_string());
        session.branch = Some("dev".to_string());
        session.context_used = Some(19.0);
        session.context_tokens = Some((189_300, 1_000_000));
        session.session_cost_usd = Some(0.1234);
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | QuotaStation · dev\n\
             ctx 19% (189.3k/1.0M) | $0.12\n\
             CLD 5h 24% (4h02m) · 7d 41% (5d) | CDX 5h 62% (2h10m)"
        );
    }

    #[test]
    fn the_branch_carries_what_the_checkout_has_not_committed_or_pushed() {
        let mut session = view(Some("Opus"), Vec::new());
        session.branch = Some("dev".to_string());
        session.worktree =
            Some(crate::git::WorkTreeStatus { changed: 3, ahead: 1, behind: 0, tracked: true });
        assert_eq!(plain(&status_line(&session)), "Opus | dev *3 \u{2191}1");

        session.worktree =
            Some(crate::git::WorkTreeStatus { changed: 0, ahead: 0, behind: 0, tracked: true });
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | dev",
            "a clean branch level with its remote says nothing more"
        );
    }

    /// Each setting cuts one thing, so turning both off is the only way to reach the row
    /// that carries nothing but this client's own model and quota.
    #[test]
    fn without_the_extra_detail_the_session_rows_go_and_the_quota_joins_the_model() {
        let mut session = view(Some("Opus"), two_providers());
        session.directory = Some("QuotaStation".to_string());
        session.context_used = Some(19.0);
        session.layout = StatusLineLayout::legacy(false, true);
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | CLD 5h 24% (4h02m) · 7d 41% (5d) | CDX 5h 62% (2h10m)",
            "no directory and no context, but every provider still reports"
        );

        session.layout = StatusLineLayout::legacy(false, false);
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | 5h 24% (4h02m) · 7d 41% (5d)",
            "the one provider left needs no name"
        );
    }

    #[test]
    fn without_the_other_providers_the_session_rows_stay_exactly_as_they_were() {
        let mut session = view(Some("Opus"), two_providers());
        session.directory = Some("QuotaStation".to_string());
        session.branch = Some("dev".to_string());
        session.context_used = Some(19.0);
        session.session_cost_usd = Some(0.1234);
        session.layout = StatusLineLayout::legacy(true, false);
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | QuotaStation · dev\n\
             ctx 19% | $0.12\n\
             5h 24% (4h02m) · 7d 41% (5d)",
            "only the second provider is gone"
        );
    }

    #[test]
    fn usage_is_coloured_on_the_thresholds_the_interface_uses() {
        let line = status_line(&view(
            None,
            vec![QuotaSegment {
                id: "quota:codex".to_string(),
                label: "CDX".to_string(),
                own: false,
                windows: vec![
                    window("5h", 12.0, None),
                    window("7d", 72.0, None),
                    window("30d", 91.0, None),
                ],
            }],
        ));
        assert_eq!(
            line,
            format!("CDX 5h {GREEN}12%{RESET} · 7d {YELLOW}72%{RESET} · 30d {RED}91%{RESET}")
        );
    }

    #[test]
    fn a_provider_whose_windows_have_all_restarted_is_left_out_rather_than_shown_empty() {
        let line = status_line(&view(
            Some("Opus"),
            vec![QuotaSegment {
                id: "quota:codex".to_string(),
                label: "CDX".to_string(),
                own: false,
                windows: vec![window("5h", 62.0, Some(-60))],
            }],
        ));
        assert_eq!(line, "Opus", "no second row at all");
    }

    /// Claude Code hands out no rate limits on an API-key sign-in, and the application may
    /// never have recorded the provider either, which leaves another provider's quota first
    /// in the list. Unnamed beside this client's own model it would be read as this client's.
    #[test]
    fn the_one_row_carries_only_this_client_however_the_list_begins() {
        let mut session = view(
            Some("Opus"),
            vec![QuotaSegment {
                id: "quota:codex".to_string(),
                label: "CDX".to_string(),
                own: false,
                windows: vec![window("5h", 62.0, Some(2 * 3_600))],
            }],
        );
        session.layout = StatusLineLayout::legacy(false, false);
        assert_eq!(plain(&status_line(&session)), "Opus", "no foreign quota at all");
        session.layout = StatusLineLayout::legacy(false, true);
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | CDX 5h 62% (2h00m)",
            "a foreign quota alone is still named"
        );
    }

    #[test]
    fn segments_can_move_between_rows_and_change_order() {
        let mut session = view(Some("Opus"), two_providers());
        session.branch = Some("dev".to_string());
        session.context_used = Some(19.0);
        let codex = session.layout.segments.iter().position(|s| s.id == "quota:codex").unwrap();
        let moved = session.layout.segments.remove(codex);
        session.layout.segments.insert(0, SegmentPlacement { row: 1, ..moved });
        session.layout.separators = SeparatorStyle::Arrow;
        assert_eq!(
            plain(&status_line(&session)),
            "CDX 5h 62% (2h10m) \u{203a} Opus \u{203a} dev\n\
             ctx 19%\n\
             CLD 5h 24% (4h02m) \u{b7} 7d 41% (5d)"
        );
    }

    #[test]
    fn a_quota_window_prints_the_parts_the_layout_asks_for() {
        let mut session = view(
            None,
            vec![QuotaSegment {
                id: "quota:codex".to_string(),
                label: "CDX".to_string(),
                own: false,
                windows: vec![QuotaWindow {
                    label: "5h".to_string(),
                    used_percent: 62.0,
                    resets_at: Some(NOW + 250 * 60),
                    window_minutes: Some(300),
                }],
            }],
        );
        session.layout.quota =
            QuotaFormat { used: false, remaining: true, countdown: false, pace: true, bar: true };
        assert_eq!(plain(&status_line(&session)), "CDX 5h 38% left\u{2191} ▰▰▰▱▱");
    }

    #[test]
    fn colour_can_be_limited_to_the_quota_or_left_out() {
        let mut session = view(
            None,
            vec![QuotaSegment {
                id: "quota:codex".to_string(),
                label: "CDX".to_string(),
                own: false,
                windows: vec![window("5h", 72.0, None)],
            }],
        );
        session.context_used = Some(95.0);
        session.layout.colour = ColourMode::QuotaOnly;
        assert_eq!(status_line(&session), format!("ctx 95%\nCDX 5h {YELLOW}72%{RESET}"));
        session.layout.colour = ColourMode::None;
        assert_eq!(status_line(&session), "ctx 95%\nCDX 5h 72%");
    }

    #[test]
    fn the_preview_is_the_line_the_layout_draws() {
        let mut layout = StatusLineLayout::default();
        for segment in &mut layout.segments {
            segment.enabled = segment.id == "model" || segment.id == "today";
            segment.row = 1;
        }
        assert_eq!(plain(&preview(layout, ProviderLabelStyle::Short)), "Opus | today 12M $18.25");
    }

    fn enable(session: &mut StatusLineView, ids: &[&str]) {
        for segment in &mut session.layout.segments {
            if ids.contains(&segment.id.as_str()) {
                segment.enabled = true;
            }
        }
    }

    #[test]
    fn the_session_detail_claude_code_reports_can_be_switched_on() {
        let mut session = view(Some("Opus"), Vec::new());
        session.session_name = Some("refactor-auth");
        session.output_style = Some("default");
        session.large_context = true;
        session.lines_changed = Some((156, 23));
        session.duration_ms = Some((45 * 60_000, Some(125_000)));
        let default = plain(&status_line(&session));
        assert_eq!(default, "Opus", "nothing new appears until it is switched on");

        enable(
            &mut session,
            &["sessionName", "outputStyle", "largeContext", "linesChanged", "duration"],
        );
        assert_eq!(
            plain(&status_line(&session)),
            "Opus | refactor-auth\nctx >200k | +156 \u{2212}23 \u{b7} 45m (api 2m)",
            "the default output style says nothing and is left out"
        );
    }

    #[test]
    fn the_project_and_quotastation_counts_can_be_switched_on() {
        let mut session = view(None, Vec::new());
        session.head = Some(crate::git::HeadCommit {
            committed_at: NOW - 3 * 3_600,
            describe: Some("v1.1.0-6-gde36776".to_string()),
        });
        session.week = Some((85_000_000, Some(120.5)));
        session.last_reset = Some(ResetNote {
            provider: "CDX".to_string(),
            window: "5h".to_string(),
            at: NOW - 2 * 3_600,
            early: true,
        });
        session.reset_credits = vec![("CDX".to_string(), 2)];
        session.quota_age = Some(240);
        enable(
            &mut session,
            &["lastCommit", "tag", "week", "lastReset", "resetCredits", "quotaAge"],
        );
        assert_eq!(
            plain(&status_line(&session)),
            "commit 3h ago \u{b7} v1.1.0+6\n\
             week 85M $120.50 | CDX 5h reset early 2h ago \u{b7} CDX credits 2 | quota read 4m ago"
        );
    }

    #[test]
    fn a_tag_description_reads_as_the_tag_and_the_commits_since() {
        assert_eq!(tag_text("v1.1.0-6-gde36776"), "v1.1.0+6");
        assert_eq!(tag_text("v1.1.0"), "v1.1.0", "a tagged commit is the tag alone");
        assert_eq!(tag_text("release-2-0"), "release-2-0", "dashes inside a tag are kept");
    }

    #[test]
    fn the_payload_windows_are_read_as_the_share_consumed() {
        let windows = windows_from_payload(Some(&RateLimits {
            five_hour: Some(Bucket { used_percentage: Some(23.5), resets_at: Some(NOW + 60) }),
            seven_day: Some(Bucket { used_percentage: None, resets_at: Some(NOW + 60) }),
        }));
        assert_eq!(windows.len(), 1, "a bucket with no percentage renders nothing");
        assert_eq!(windows[0].label, "5h");
        assert_eq!(windows[0].used_percent, 23.5);
    }

    #[test]
    fn the_session_row_carries_what_the_session_is_set_to_do() {
        let mut session = view(Some("Opus"), Vec::new());
        session.effort = Some("high");
        session.thinking = true;
        session.fast_mode = true;
        session.pull_request = Some((1234, Some("pending")));
        session.cache_hit = Some(78.4);
        assert_eq!(
            plain(&status_line(&session)),
            "Opus · high · fast · think | PR #1234 pending
cache 78.4%"
        );
    }

    #[test]
    fn the_cache_share_counts_every_input_token_the_response_reported() {
        let usage = CurrentUsage {
            input_tokens: Some(1_000),
            cache_creation_input_tokens: Some(1_000),
            cache_read_input_tokens: Some(8_000),
        };
        assert_eq!(cache_hit(&usage), Some(80.0));
        let empty = CurrentUsage {
            input_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        assert_eq!(cache_hit(&empty), None, "nothing reported is not a zero hit rate");
    }

    #[test]
    fn each_pace_a_window_can_be_at_is_drawn_as_its_own_marker() {
        // Half of a five-hour window has passed.
        let halfway = |used| QuotaWindow {
            label: "5h".to_string(),
            used_percent: used,
            resets_at: Some(NOW + 150 * 60),
            window_minutes: Some(300),
        };
        assert_eq!(pace_marker(&halfway(80.0), NOW), Some("↑"), "spent well ahead of time");
        assert_eq!(pace_marker(&halfway(20.0), NOW), Some("↓"), "spent well behind time");
        assert_eq!(pace_marker(&halfway(55.0), NOW), None, "inside the band");
    }

    #[test]
    fn token_counts_are_written_the_way_the_context_command_writes_them() {
        assert_eq!(tokens_short(189_300), "189.3k");
        assert_eq!(tokens_short(1_000_000), "1.0M");
        assert_eq!(tokens_short(200_000), "200.0k");
        assert_eq!(tokens_short(940), "940");
    }

    #[test]
    fn the_context_window_is_read_as_the_tokens_now_in_it() {
        let payload = r#"{
            "context_window": {
                "total_input_tokens": 15500,
                "total_output_tokens": 1200,
                "context_window_size": 200000,
                "used_percentage": 8
            }
        }"#;
        let input: StatusLineInput = serde_json::from_str(payload).expect("read the payload");
        let context = input.context_window.expect("the payload carries a context window");
        assert_eq!(context.total_input_tokens, Some(15_500));
        assert_eq!(context.context_window_size, Some(200_000));
    }

    #[test]
    fn the_installed_command_is_recognised_however_the_path_is_quoted() {
        assert!(is_bridge_command(
            "\"C:\\Program Files\\QuotaStation\\quotastation.exe\" --claude-statusline"
        ));
        assert!(!is_bridge_command("npx ccusage statusline"));
    }

    fn scratch_settings(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("quotastation-{name}-settings.json"));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn installing_adds_the_status_line_and_leaves_every_other_setting_alone() {
        let path = scratch_settings("install");
        std::fs::write(&path, r#"{"env":{"A":"1"},"inputNeededNotifEnabled":true}"#).unwrap();
        install_into(&path, "\"q.exe\" --claude-statusline").expect("install the status line");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["type"], "command");
        assert_eq!(settings["statusLine"]["command"], "\"q.exe\" --claude-statusline");
        assert_eq!(settings["env"]["A"], "1", "unrelated settings survive the write");
        assert_eq!(settings["inputNeededNotifEnabled"], true);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.find("\"env\"").unwrap() < written.find("\"inputNeededNotifEnabled\"").unwrap()
        );

        remove_from(&path).expect("remove the status line");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(settings.get("statusLine").is_none());
        assert_eq!(settings["env"]["A"], "1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_status_line_belonging_to_something_else_is_never_replaced_or_removed() {
        let path = scratch_settings("foreign");
        let foreign = r#"{"statusLine":{"type":"command","command":"npx ccusage statusline"}}"#;
        std::fs::write(&path, foreign).unwrap();
        assert!(install_into(&path, "\"q.exe\" --claude-statusline").is_err());
        remove_from(&path).expect("removing leaves a foreign status line in place");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["command"], "npx ccusage statusline");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_settings_file_is_written_from_nothing() {
        let path = scratch_settings("absent");
        install_into(&path, "\"q.exe\" --claude-statusline").expect("install the status line");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["command"], "\"q.exe\" --claude-statusline");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_concurrent_settings_change_is_not_overwritten() {
        let path = scratch_settings("concurrent");
        std::fs::write(&path, r#"{"first":true}"#).unwrap();
        let (_, original) = load_settings_with_source(&path).expect("read original settings");
        std::fs::write(&path, r#"{"changed":true}"#).unwrap();
        let replacement = serde_json::json!({ "statusLine": { "command": "q" } });
        assert!(write_settings(&path, &replacement, original.as_deref()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"changed":true}"#);
        let _ = std::fs::remove_file(&path);
    }

    /// The payload Claude Code documents, read exactly as it arrives.
    #[test]
    fn the_documented_payload_is_read_as_two_windows() {
        let payload = r#"{
            "model": { "id": "claude-opus-5", "display_name": "Opus" },
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
                "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 }
            }
        }"#;
        let input: StatusLineInput = serde_json::from_str(payload).expect("read the payload");
        let limits = input.rate_limits.expect("the payload carries rate limits");
        assert_eq!(
            limits.five_hour,
            Some(Bucket { used_percentage: Some(23.5), resets_at: Some(1_738_425_600) })
        );
        assert_eq!(
            limits.seven_day,
            Some(Bucket { used_percentage: Some(41.2), resets_at: Some(1_738_857_600) })
        );
    }
}
