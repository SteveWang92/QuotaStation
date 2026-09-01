//! Desktop notifications for the things that change while nobody is watching the window.
//!
//! QuotaStation runs all day with no window open, so a quota window that runs out, an
//! acquisition path that stops answering, and a window that restarts are all invisible until
//! someone goes looking. Each of those is announced once, when it happens, and never again
//! until the condition that raised it has cleared — a notification repeated on a two-minute
//! refresh loop is a notification that gets turned off.
//!
//! The thresholds are not new ones: a window is announced when it crosses into the same
//! warning and critical shares every surface already colours it by.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use crate::{
    domain::{
        CRITICAL_PERCENT, CompactStatusLevel, Freshness, LimitKind, LimitWindow, ProviderSnapshot,
        ResetClassification, WARNING_PERCENT, WorkspaceSnapshot,
    },
    providers::ProviderKind,
    settings::AppSettings,
};

/// One notification, ready to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

/// How loud a window's own reading is, in the shares the rest of the application already
/// draws it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Loudness {
    #[default]
    Quiet,
    Warning,
    Critical,
}

impl Loudness {
    fn of(window: &LimitWindow) -> Self {
        match window.used_percent {
            Some(percent) if percent >= CRITICAL_PERCENT => Self::Critical,
            Some(percent) if percent >= WARNING_PERCENT => Self::Warning,
            _ => Self::Quiet,
        }
    }
}

/// What the loudest reading already announced for one quota window was, and which run of
/// that window it belonged to. A confirmed restart re-arms it: the same window at 95%
/// before and after a reset is two different facts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuotaMark {
    /// The newest confirmed restart of this kind. A published expiry can move while a
    /// rolling window merely ages out old requests, so it cannot identify a new run.
    reset_anchor: Option<i64>,
    /// The loudest reading announced for the current run of this window.
    announced: Loudness,
    /// The previous reading itself, which is what makes an alert a crossing rather than a
    /// restatement of something already true.
    observed: Loudness,
}

/// Everything already said, so nothing is said twice.
#[derive(Debug, Default, PartialEq)]
pub struct Announced {
    quota: HashMap<(ProviderKind, LimitKind), QuotaMark>,
    /// The providers currently known to be failing. Recovering removes one, which is what
    /// allows the next failure to be announced.
    failing: Vec<ProviderKind>,
    /// The newest restart already announced per provider, by the moment the restarted
    /// window began.
    resets: HashMap<ProviderKind, i64>,
    /// The first review after a start records the present without announcing it. Everything
    /// it would otherwise report is already true — the machine has just been turned on, and
    /// a quota that was low yesterday is not news this morning.
    seeded: bool,
}

/// The alerts a new snapshot has earned, and the record of them.
pub fn pending(
    announced: &mut Announced,
    workspace: &WorkspaceSnapshot,
    settings: &AppSettings,
) -> Vec<Alert> {
    let seeding = !announced.seeded;
    announced.seeded = true;
    let mut alerts = Vec::new();
    for provider in &workspace.providers {
        if provider.remote_usage_only {
            continue;
        }
        collect_quota(announced, provider, settings, seeding, &mut alerts);
        collect_failure(announced, provider, settings, seeding, &mut alerts);
        collect_resets(announced, provider, settings, seeding, &mut alerts);
    }
    alerts
}

fn collect_quota(
    announced: &mut Announced,
    provider: &ProviderSnapshot,
    settings: &AppSettings,
    seeding: bool,
    alerts: &mut Vec<Alert>,
) {
    for window in &provider.limits {
        // A reading that is no longer current cannot raise an alarm. It is the same reading
        // that was already judged when it was fresh, and the provider's own failure alert
        // covers the fact that nothing newer has arrived.
        if window.freshness != Freshness::Fresh {
            continue;
        }
        let loudness = Loudness::of(window);
        let key = (provider.provider, window.kind);
        let reset_anchor = provider
            .recent_resets
            .iter()
            .find(|reset| reset.window_kind == window.kind)
            .map(|reset| reset.anchored_at);
        let previous = announced.quota.get(&key);
        // An anchor that appears, or that moves forward, is a new run of the window and
        // re-arms the alert. An anchor that disappears is not: the provider carries a
        // bounded list of recent restarts, so a busy five-hour window can push the weekly
        // window's restart off the end of it while that window runs on unchanged.
        let seen_anchor = previous.and_then(|mark| mark.reset_anchor);
        let restarted = match (reset_anchor, seen_anchor) {
            (Some(now), Some(seen)) => now > seen,
            (Some(_), None) => true,
            _ => false,
        };
        let carried =
            previous.filter(|_| !restarted).map(|mark| mark.announced).unwrap_or_default();
        let observed = previous.map(|mark| mark.observed).unwrap_or_default();
        announced.quota.insert(
            key,
            QuotaMark {
                reset_anchor: reset_anchor.max(seen_anchor),
                announced: loudness.max(carried),
                observed: loudness,
            },
        );
        // Only a reading that has climbed is worth announcing, whatever the bookkeeping
        // around it did. The reset backfill puts the running window's first anchor on
        // record minutes after the reading itself arrived, and re-arming on that alone
        // would announce a share that has not moved since the start this run deliberately
        // stayed quiet about.
        if seeding || !settings.notify_low_quota || loudness <= carried || loudness <= observed {
            continue;
        }
        alerts.push(quota_alert(provider, window, loudness));
    }
}

fn quota_alert(provider: &ProviderSnapshot, window: &LimitWindow, loudness: Loudness) -> Alert {
    let title = match loudness {
        Loudness::Critical => format!("{} quota nearly gone", provider.display_name),
        _ => format!("{} quota running low", provider.display_name),
    };
    let used = window.used_percent.unwrap();
    let body = match window.resets_at.and_then(local_moment) {
        Some(moment) => format!("{} {used:.0}% used · resets {moment}", window.label),
        None => format!("{} {used:.0}% used", window.label),
    };
    Alert { title, body }
}

fn collect_failure(
    announced: &mut Announced,
    provider: &ProviderSnapshot,
    settings: &AppSettings,
    seeding: bool,
    alerts: &mut Vec<Alert>,
) {
    let key = provider.provider;
    let was_failing = announced.failing.contains(&key);
    let Some(reason) = failure_reason(provider) else {
        announced.failing.retain(|failing| failing != &key);
        return;
    };
    if !was_failing {
        announced.failing.push(key);
    }
    if seeding || was_failing || !settings.notify_read_failures {
        return;
    }
    alerts.push(Alert { title: format!("{} cannot be read", provider.display_name), body: reason });
}

/// Why a provider is in trouble, or `None` while it is answering.
///
/// The status level is the one every surface already uses, so a notification and the tray
/// icon can never disagree about whether something is wrong. Errors reaching here have been
/// sanitized by the core, so they are safe to show.
fn failure_reason(provider: &ProviderSnapshot) -> Option<String> {
    // Neither of these is a provider that cannot be read. One is waiting to be signed in
    // again, which the dashboard and tray already say; the other is quota nobody asked for.
    if provider.sign_in_required || provider.quota_disabled {
        return None;
    }
    match provider.compact_status.level {
        CompactStatusLevel::Unavailable | CompactStatusLevel::Stale => Some(
            provider
                .live_error
                .clone()
                .or_else(|| provider.history_error.clone())
                .unwrap_or_else(|| provider.compact_status.label.clone()),
        ),
        _ => None,
    }
}

fn collect_resets(
    announced: &mut Announced,
    provider: &ProviderSnapshot,
    settings: &AppSettings,
    seeding: bool,
    alerts: &mut Vec<Alert>,
) {
    let Some(reset) = provider.recent_resets.first() else { return };
    let previous = announced.resets.insert(provider.provider, reset.anchored_at);
    if seeding
        || !settings.notify_quota_resets
        || previous.is_some_and(|at| at >= reset.anchored_at)
    {
        return;
    }
    let early = match reset.classification {
        ResetClassification::Unplanned => " early",
        ResetClassification::Scheduled => "",
    };
    let body = match local_moment(reset.new_resets_at) {
        Some(moment) => format!("{} restarted{early} · next reset {moment}", reset.window_label),
        None => format!("{} restarted{early}", reset.window_label),
    };
    alerts.push(Alert { title: format!("{} quota reset", provider.display_name), body });
}

/// When something happens, in local time on the 24-hour clock every other surface uses.
///
/// The date is carried whenever it is not today's. A weekly window restarting "at 22:30" is
/// the wrong answer four days early, and a notification is read once with no window beside it
/// to check against.
fn local_moment(epoch: i64) -> Option<String> {
    written_moment(epoch, jiff::Timestamp::now().as_second())
}

fn written_moment(epoch: i64, now: i64) -> Option<String> {
    let zone = jiff::tz::TimeZone::system();
    let moment = jiff::Timestamp::from_second(epoch).ok()?.to_zoned(zone.clone());
    let today = jiff::Timestamp::from_second(now).ok()?.to_zoned(zone);
    let format = match moment.date() == today.date() {
        true => "at %H:%M",
        false => "on %b %d at %H:%M",
    };
    Some(moment.strftime(format).to_string())
}

fn announced() -> &'static Mutex<Announced> {
    static ANNOUNCED: OnceLock<Mutex<Announced>> = OnceLock::new();
    ANNOUNCED.get_or_init(|| Mutex::new(Announced::default()))
}

/// Shows whatever a freshly published snapshot has earned.
pub fn review(app: &tauri::AppHandle, workspace: &WorkspaceSnapshot) {
    use tauri::Manager;

    let Some(state) = app.try_state::<std::sync::Arc<crate::AppState>>() else { return };
    let settings = state.settings();
    let alerts = {
        let Ok(mut record) = announced().lock() else { return };
        pending(&mut record, workspace, &settings)
    };
    for alert in alerts {
        raise(app, &alert.title, &alert.body);
    }
}

/// Shows one desktop notification.
///
/// Windows refuses a toast from an application it cannot identify, and refuses it silently:
/// from the user's side the event happens and nothing appears, with nowhere to look. The log
/// is that place.
pub fn raise(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;

    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        crate::log::write(format!("desktop notification failed: {error}"));
    }
}

/// The same notification, with somewhere for a click to go.
///
/// The notification plugin shows a toast and returns; it has no way to hear that one was
/// clicked on Windows, so a toast that answers a click has to be raised through the WinRT
/// API directly. The application identifier is the same one the plugin uses, so the toast
/// still arrives under QuotaStation's own name in the action centre.
///
/// The activation only reaches a running process. That is exactly the case here — the
/// application is what raised the toast — and anything that goes wrong falls back to an
/// ordinary notification rather than to none.
#[cfg(windows)]
pub fn raise_with_action(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
    on_click: impl Fn() + Send + 'static,
) {
    use tauri_winrt_notification::Toast;

    let identifier = app.config().identifier.clone();
    let shown = Toast::new(&identifier)
        .title(title)
        .text1(body)
        .on_activated(move |_action| {
            on_click();
            Ok(())
        })
        .show();
    if let Err(error) = shown {
        crate::log::write(format!("clickable notification failed: {error}"));
        raise(app, title, body);
    }
}

#[cfg(not(windows))]
pub fn raise_with_action(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
    _on_click: impl Fn() + Send + 'static,
) {
    raise(app, title, body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CompactStatus, LimitResetEvent, QuotaLevel, WindowSource};

    fn window(kind: LimitKind, used: f64, resets_at: i64) -> LimitWindow {
        LimitWindow {
            kind,
            label: "5h window".to_string(),
            used_percent: Some(used),
            window_duration_mins: Some(300),
            resets_at: Some(resets_at),
            source: WindowSource::AppServer,
            observed_at: 1_800_000_000,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
        }
    }

    fn provider(limits: Vec<LimitWindow>) -> ProviderSnapshot {
        let mut snapshot = ProviderSnapshot::new(ProviderKind::Codex);
        snapshot.limits = limits;
        snapshot
    }

    /// A provider that is answering normally.
    fn answering() -> ProviderSnapshot {
        let mut snapshot = provider(Vec::new());
        snapshot.compact_status =
            CompactStatus { level: CompactStatusLevel::Healthy, label: "Healthy".to_string() };
        snapshot
    }

    fn workspace(provider: ProviderSnapshot) -> WorkspaceSnapshot {
        WorkspaceSnapshot::new(vec![provider])
    }

    /// The first review after a start is a baseline, not an announcement: the application
    /// starts with Windows, and greeting every login with yesterday's quota is how a
    /// notification earns itself a permanent "off".
    #[test]
    fn the_state_at_startup_is_recorded_rather_than_announced() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        let full = workspace(provider(vec![window(LimitKind::Primary, 96.0, 1_800_003_600)]));
        assert!(pending(&mut announced, &full, &settings).is_empty());
    }

    #[test]
    fn a_window_is_announced_once_per_threshold_it_crosses() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 10.0, 1_800_003_600)])),
            &settings,
        );

        let warning = pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 72.0, 1_800_003_600)])),
            &settings,
        );
        assert_eq!(warning.len(), 1, "crossing into the warning share is announced");
        assert!(warning[0].title.contains("running low"));
        assert!(
            warning[0].body.contains("72%"),
            "the body carries the reading: {}",
            warning[0].body
        );

        let again = pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 80.0, 1_800_003_600)])),
            &settings,
        );
        assert!(again.is_empty(), "rising within the same share says nothing further");

        let critical = pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 94.0, 1_800_003_600)])),
            &settings,
        );
        assert_eq!(critical.len(), 1, "the louder threshold is its own crossing");
        assert!(critical[0].title.contains("nearly gone"));
    }

    /// A weekly window restarts on a day, not at a time of day. Without the date, "resets at
    /// 22:30" reads as tonight.
    #[test]
    fn a_reset_more_than_a_day_out_is_written_with_its_date() {
        let now = 1_800_000_000;
        let today = written_moment(now + 3_600, now).expect("today's reset");
        assert!(today.starts_with("at "), "today needs no date: {today}");
        let later = written_moment(now + 4 * 86_400, now).expect("a reset four days out");
        assert!(later.starts_with("on "), "a later day carries its date: {later}");
        assert!(later.contains(" at "), "and still names the time: {later}");
    }

    #[test]
    fn a_moving_expiry_does_not_rearm_a_rolling_window() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 10.0, 1_800_003_600)])),
            &settings,
        );
        pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 94.0, 1_800_003_600)])),
            &settings,
        );
        let moved = pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 94.0, 1_800_021_600)])),
            &settings,
        );
        assert!(
            moved.is_empty(),
            "an expiry shift without a confirmed restart is still the same window"
        );

        // The window's own restart history has to be on record before a later restart can
        // be told apart from a history that has only just been read.
        let restart = |anchored_at: i64| LimitResetEvent {
            window_kind: LimitKind::Primary,
            window_label: "5h window".to_string(),
            window_duration_mins: 300,
            anchored_at,
            new_resets_at: anchored_at + 18_000,
            previous_resets_at: anchored_at,
            used_percent_before: 94.0,
            early_by_seconds: 0,
            tokens_in_window: None,
            classification: ResetClassification::Scheduled,
        };
        // A restart is recognised from the collapse in the share itself, so the reading at
        // the restart is always a low one; the window filling up again is what earns the
        // alert a second time.
        let mut restarted = provider(vec![window(LimitKind::Primary, 4.0, 1_800_039_600)]);
        restarted.recent_resets = vec![restart(1_800_021_600)];
        let at_restart = pending(&mut announced, &workspace(restarted.clone()), &settings);
        assert_eq!(at_restart.len(), 1, "the restart itself is the only news it carries");
        assert!(at_restart[0].title.contains("quota reset"));

        let mut refilled = restarted;
        refilled.limits = vec![window(LimitKind::Primary, 94.0, 1_800_039_600)];
        let after_reset = pending(&mut announced, &workspace(refilled), &settings);
        assert_eq!(after_reset.len(), 1, "the new window filling up is a new fact");
        assert!(after_reset[0].title.contains("quota nearly gone"));
    }

    #[test]
    fn a_stale_reading_raises_nothing_of_its_own() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 10.0, 1_800_003_600)])),
            &settings,
        );
        let mut stale = window(LimitKind::Primary, 94.0, 1_800_003_600);
        stale.freshness = Freshness::Stale;
        assert!(pending(&mut announced, &workspace(provider(vec![stale])), &settings).is_empty());
    }

    #[test]
    fn a_failing_provider_is_announced_once_and_again_only_after_it_recovers() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        // A provider with nothing read yet is already unavailable, which is the state this
        // starts from: the baseline has to be one that is answering.
        pending(&mut announced, &workspace(answering()), &settings);

        let mut broken = provider(Vec::new());
        broken.compact_status = CompactStatus {
            level: CompactStatusLevel::Unavailable,
            label: "Provider unavailable".to_string(),
        };
        broken.live_error = Some("Quota read failed".to_string());
        let first = pending(&mut announced, &workspace(broken.clone()), &settings);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].body, "Quota read failed");
        assert!(
            pending(&mut announced, &workspace(broken.clone()), &settings).is_empty(),
            "a failure that is still failing is not news"
        );

        assert!(pending(&mut announced, &workspace(answering()), &settings).is_empty());
        assert_eq!(
            pending(&mut announced, &workspace(broken), &settings).len(),
            1,
            "failing again after a recovery is a new failure"
        );
    }

    #[test]
    fn a_provider_with_no_quota_to_read_is_not_announced_as_a_read_failure() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();

        let mut signed_out = provider(Vec::new());
        signed_out.sign_in_required = true;
        signed_out.resolve_derived_state();
        pending(&mut announced, &workspace(answering()), &settings);
        assert!(
            pending(&mut announced, &workspace(signed_out), &settings).is_empty(),
            "a client waiting to be signed in again is not a source that broke"
        );

        let mut untracked = provider(Vec::new());
        untracked.quota_disabled = true;
        untracked.resolve_derived_state();
        let mut announced = Announced::default();
        pending(&mut announced, &workspace(answering()), &settings);
        assert!(
            pending(&mut announced, &workspace(untracked), &settings).is_empty(),
            "quota nobody asked for cannot fail to arrive"
        );
    }

    #[test]
    fn a_restart_ageing_out_of_the_recent_list_does_not_rearm_the_alert() {
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        let restart = LimitResetEvent {
            window_kind: LimitKind::Secondary,
            window_label: "Weekly window".to_string(),
            window_duration_mins: 10_080,
            anchored_at: 1_800_000_000,
            new_resets_at: 1_800_604_800,
            previous_resets_at: 1_800_000_000,
            used_percent_before: 91.0,
            early_by_seconds: 0,
            tokens_in_window: None,
            classification: ResetClassification::Scheduled,
        };
        let mut restarted = provider(vec![window(LimitKind::Secondary, 20.0, 1_800_604_800)]);
        restarted.recent_resets = vec![restart];
        pending(&mut announced, &workspace(restarted.clone()), &settings);
        let mut filling = restarted.clone();
        filling.limits = vec![window(LimitKind::Secondary, 94.0, 1_800_604_800)];
        assert_eq!(
            pending(&mut announced, &workspace(filling.clone()), &settings).len(),
            1,
            "the window filling up is announced once"
        );
        // Enough five-hour restarts have been recorded since to push this one off the end
        // of the provider's recent list.
        let mut aged_out = filling;
        aged_out.recent_resets = Vec::new();
        assert!(
            pending(&mut announced, &workspace(aged_out), &settings).is_empty(),
            "losing sight of the restart does not make the same window new again"
        );
    }

    #[test]
    fn only_a_restart_newer_than_the_last_one_announced_is_reported() {
        let reset = |anchored_at: i64| LimitResetEvent {
            window_kind: LimitKind::Primary,
            window_label: "5h window".to_string(),
            window_duration_mins: 300,
            anchored_at,
            new_resets_at: anchored_at + 18_000,
            previous_resets_at: anchored_at,
            used_percent_before: 88.0,
            early_by_seconds: 0,
            tokens_in_window: None,
            classification: ResetClassification::Scheduled,
        };
        let mut announced = Announced::default();
        let settings = AppSettings::default();
        let mut first = provider(Vec::new());
        first.recent_resets = vec![reset(1_800_000_000)];
        assert!(
            pending(&mut announced, &workspace(first.clone()), &settings).is_empty(),
            "the restarts already on record at startup are history"
        );
        assert!(pending(&mut announced, &workspace(first), &settings).is_empty());

        let mut next = provider(Vec::new());
        next.recent_resets = vec![reset(1_800_018_000)];
        let alerts = pending(&mut announced, &workspace(next), &settings);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].title.contains("quota reset"));
    }

    #[test]
    fn remote_usage_is_neither_unavailable_nor_a_read_failure() {
        let mut remote = provider(Vec::new());
        remote.remote_usage_only = true;
        remote.resolve_derived_state();
        assert_eq!(remote.compact_status.level, CompactStatusLevel::Healthy);

        let mut announced = Announced::default();
        let settings = AppSettings::default();
        pending(&mut announced, &workspace(answering()), &settings);
        remote.compact_status = CompactStatus {
            level: CompactStatusLevel::Unavailable,
            label: "Provider unavailable".to_string(),
        };
        remote.live_error = Some("Quota read failed".to_string());
        assert!(
            pending(&mut announced, &workspace(remote), &settings).is_empty(),
            "a provider whose quota belongs to another device must not raise a failure alert"
        );
    }

    #[test]
    fn each_kind_of_alert_can_be_turned_off_on_its_own() {
        let mut announced = Announced::default();
        let settings = AppSettings { notify_low_quota: false, ..AppSettings::default() };
        pending(
            &mut announced,
            &workspace(provider(vec![window(LimitKind::Primary, 10.0, 1_800_003_600)])),
            &settings,
        );
        assert!(
            pending(
                &mut announced,
                &workspace(provider(vec![window(LimitKind::Primary, 96.0, 1_800_003_600)])),
                &settings,
            )
            .is_empty()
        );
    }
}
