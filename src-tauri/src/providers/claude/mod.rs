mod api;
mod history;
mod session;
pub mod statusline;

use std::{env, path::PathBuf};

use anyhow::Result;

use crate::domain::{CrossCheck, LimitKind, LimitWindow, LiveSnapshot};

pub use history::read_history;

/// Claude's rolling session window, in minutes.
pub const FIVE_HOUR_WINDOW_MINS: i64 = 300;
/// Claude's long window is seven days, which `LimitKind::window_label` names "Weekly".
pub const SEVEN_DAY_WINDOW_MINS: i64 = 10_080;

/// The directory Claude Code keeps its configuration, credentials, and session logs in.
///
/// `CLAUDE_CONFIG_DIR` may list several directories; everything QuotaStation reads lives
/// with the first one, matching how the log parser resolves its own paths.
pub fn claude_home() -> Option<PathBuf> {
    if let Some(configured) = env::var_os("CLAUDE_CONFIG_DIR") {
        let configured = configured.to_string_lossy().into_owned();
        if let Some(first) = configured.split(',').map(str::trim).find(|value| !value.is_empty()) {
            return Some(PathBuf::from(first));
        }
    }
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")).map(PathBuf::from)?;
    Some(home.join(".claude"))
}

/// Claude quota, from the three sources this machine can offer, in order of what each one
/// actually knows.
///
/// Claude Code's own status line is the best of them: when the bridge is installed it hands
/// over both windows with the percentage consumed and the exact restart, needs no
/// credential, and shares no rate limit. It only speaks while Claude Code is running, so
/// the session logs remain underneath it — they are written whatever else is configured,
/// and they give the five-hour window's timing. Anthropic's usage endpoint stays available
/// as an opt-in correction for installations without the bridge.
pub async fn read_live(cross_check: bool) -> Result<LiveSnapshot> {
    let plan_type = api::plan_type();
    let reported = statusline::read_windows();
    let mut snapshot = match session::read_live(plan_type.clone()).await {
        Ok(snapshot) => snapshot,
        // The session logs are the fallback, so their absence only ends the read when
        // nothing else answered either.
        Err(error) if reported.is_empty() => return Err(error),
        Err(_) => LiveSnapshot {
            plan_type,
            limits: Vec::new(),
            earned_reset_count: None,
            cross_check: CrossCheck::NotAttempted,
        },
    };
    snapshot.limits = merge_windows(reported, snapshot.limits);
    if !cross_check {
        return Ok(snapshot);
    }
    match api::read_windows().await {
        Ok(limits) if !limits.is_empty() => {
            snapshot.limits = merge_windows(limits, snapshot.limits);
            snapshot.cross_check = CrossCheck::Confirmed;
        }
        Ok(_) => {
            snapshot.cross_check =
                CrossCheck::Failed("The usage endpoint reported no quota windows.".to_string());
        }
        Err(error) => snapshot.cross_check = CrossCheck::Failed(error.to_string()),
    }
    Ok(snapshot)
}

/// Combines two readings of the same windows, where `preferred` is the better-informed
/// source. A window only one source knows about is kept: the sources describe the same two
/// windows but not always both of them, and dropping a known window would hide a limit that
/// is really there.
fn merge_windows(preferred: Vec<LimitWindow>, fallback: Vec<LimitWindow>) -> Vec<LimitWindow> {
    let mut merged = preferred;
    for window in fallback {
        if !merged.iter().any(|kept| kept.kind == window.kind) {
            merged.push(window);
        }
    }
    // Declaration order of the kinds is display order on every surface.
    merged.sort_by_key(|window| match window.kind {
        LimitKind::Primary => 0,
        LimitKind::Secondary => 1,
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(kind: LimitKind, used_percent: Option<f64>, resets_at: i64) -> LimitWindow {
        LimitWindow {
            kind,
            label: kind.window_label(Some(FIVE_HOUR_WINDOW_MINS)),
            used_percent,
            remaining_percent: used_percent.map(|used| 100.0 - used),
            window_duration_mins: Some(FIVE_HOUR_WINDOW_MINS),
            resets_at: Some(resets_at),
        }
    }

    #[test]
    fn the_better_informed_source_owns_a_window_both_report() {
        let merged = merge_windows(
            vec![window(LimitKind::Primary, Some(30.0), 100)],
            vec![window(LimitKind::Primary, None, 200)],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].used_percent, Some(30.0));
        assert_eq!(merged[0].resets_at, Some(100));
    }

    #[test]
    fn a_window_only_the_fallback_knows_survives_the_merge() {
        let merged = merge_windows(
            vec![window(LimitKind::Secondary, Some(12.0), 500)],
            vec![window(LimitKind::Primary, None, 200)],
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].kind, LimitKind::Primary, "windows stay in display order");
        assert_eq!(merged[1].kind, LimitKind::Secondary);
    }

    #[test]
    fn a_fallback_alone_is_still_a_reading() {
        let merged = merge_windows(Vec::new(), vec![window(LimitKind::Primary, None, 200)]);
        assert_eq!(merged.len(), 1);
    }
}
