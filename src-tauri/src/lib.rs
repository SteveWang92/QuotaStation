mod alerts;
mod autostart;
mod clock;
mod commands;
mod diagnostic_export;
mod fs_atomic;
mod git;
mod log;
mod quick_panel;
mod refresh;
mod reinstall;
mod sanitize;
mod session_watcher;
mod shell;
mod summary;
mod sync;
mod taskbar;
mod terminal;
mod theme;
mod tray;

// The application is the only consumer of this library, with one exception: the
// `seed_demo` example fills the demonstration database described in `demo`, and writes it
// through the same storage, domain and settings code the application itself uses rather
// than through a second copy of the schema. These modules are public for that example.
pub mod demo;
pub mod domain;
pub mod providers;
pub mod resets;
pub mod settings;
pub mod storage;

use crate::commands::on_off;
use crate::settings::AppSettings;
use crate::tray::{build_tray, show_main};

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::{Duration, Instant},
};

use domain::{ProviderSnapshot, SharedFolderDiagnostics, WatcherDiagnostics, WorkspaceSnapshot};
use providers::{ProviderKind, claude::notifications, claude::statusline};
use storage::Storage;

/// The NSIS uninstaller's private entry point. It is deliberately not a general cleanup
/// command: the updater's `/UPDATE` uninstall keeps every integration, and a normal launch
/// must never change another program's settings.
const UNINSTALL_CLEANUP_ARG: &str = "--uninstall-cleanup";

/// Removes the external commands QuotaStation registered in Claude Code, and reports an
/// exit code only when this process was started by the uninstaller.
///
/// Both removers inspect the current Claude Code settings and delete only commands carrying
/// QuotaStation's own arguments. A malformed or concurrently changed file fails visibly to
/// NSIS instead of replacing the file or disturbing another hook. What was on is written
/// down first, so a reinstall puts the same integrations back — see [`reinstall`].
pub fn run_uninstall_cleanup() -> Option<i32> {
    if !std::env::args_os().any(|argument| argument == UNINSTALL_CLEANUP_ARG) {
        return None;
    }
    reinstall::record_uninstall();
    let mut failed = false;
    for (name, result) in [
        ("Claude Code status line", statusline::remove()),
        ("Claude Code notification hook", notifications::remove()),
    ] {
        match result {
            Ok(()) => log::write(format!("uninstall removed the {name}")),
            Err(error) => {
                failed = true;
                log::write(format!("uninstall could not remove the {name}: {error:#}"));
            }
        }
    }
    Some(if failed { 1 } else { 0 })
}

/// Gives this machine an identity in a shared usage folder if it has not got one yet.
///
/// Generated once and then kept for good: another machine stores this machine's aggregates
/// under this identifier, so issuing a new one would orphan every row it has for us. The
/// name beside it is only a label and defaults to what Windows calls the computer.
fn ensure_device_identity(path: &std::path::Path, mut settings: AppSettings) -> AppSettings {
    if settings.device_id.is_some() && settings.device_name.is_some() {
        return settings;
    }
    settings.device_id.get_or_insert_with(settings::new_device_id);
    settings.device_name.get_or_insert_with(settings::default_device_name);
    if let Err(error) = settings::save(path, &settings) {
        log::write(format!("this machine's device identity could not be recorded: {error}"));
    }
    settings
}

/// Runs whichever Claude Code hook this process was started as, and reports whether it ran
/// one. Exposed so `main` can return before any window exists.
pub fn run_claude_hook() -> bool {
    statusline::run_bridge_if_requested() || notifications::run_hook_if_requested()
}
use tauri::{Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

/// The handle the taskbar click watch reaches the application through: it is called from a
/// system mouse hook, which is a bare C callback with nowhere to carry one.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

pub struct AppState {
    storage: Storage,
    snapshots: RwLock<BTreeMap<ProviderKind, ProviderSnapshot>>,
    refresh_publish_lock: Mutex<()>,
    live_refresh_lock: Mutex<()>,
    history_refresh_lock: Mutex<()>,
    watcher_diagnostics: RwLock<WatcherDiagnostics>,
    /// What the shared usage folder last did, written by the refresh that ran it.
    shared_folder_diagnostics: RwLock<SharedFolderDiagnostics>,
    quick_panel: StdMutex<QuickPanelTiming>,
    settings: StdMutex<AppSettings>,
    detected_providers: StdMutex<Vec<ProviderKind>>,
    settings_path: PathBuf,
}

/// When the quick panel last changed hands, which is what tells one click reaching it twice,
/// or a focus loss caused by the click that opened it, apart from a real request.
#[derive(Default)]
struct QuickPanelTiming {
    focus_lost_at: Option<Instant>,
    toggled_at: Option<Instant>,
    shown_at: Option<Instant>,
}

impl AppState {
    fn new(
        storage: Storage,
        settings: AppSettings,
        settings_path: PathBuf,
        snapshots: BTreeMap<ProviderKind, ProviderSnapshot>,
        providers: Vec<ProviderKind>,
    ) -> Self {
        Self {
            storage,
            snapshots: RwLock::new(snapshots),
            refresh_publish_lock: Mutex::new(()),
            live_refresh_lock: Mutex::new(()),
            history_refresh_lock: Mutex::new(()),
            watcher_diagnostics: RwLock::new(WatcherDiagnostics::default()),
            shared_folder_diagnostics: RwLock::new(SharedFolderDiagnostics::default()),
            quick_panel: StdMutex::new(QuickPanelTiming::default()),
            settings: StdMutex::new(settings),
            detected_providers: StdMutex::new(providers),
            settings_path,
        }
    }

    /// The state a test drives, over a throwaway database and with no window behind it.
    /// Only the parts the refresh path reads are populated.
    #[cfg(test)]
    pub(crate) fn for_tests(storage: Storage) -> Self {
        Self::new(storage, AppSettings::default(), PathBuf::new(), BTreeMap::new(), Vec::new())
    }

    fn settings(&self) -> AppSettings {
        self.settings.lock().map(|settings| settings.clone()).unwrap_or_default()
    }

    /// Applies a change and records it, so a preference the user expressed survives the
    /// next start whether it came from the tray or from the settings dialog.
    fn update_settings(
        &self,
        change: impl FnOnce(&mut AppSettings),
    ) -> Result<AppSettings, String> {
        let mut settings = self.settings.lock().map_err(|_| "Settings unavailable.".to_string())?;
        let mut updated = settings.clone();
        change(&mut updated);
        settings::save(&self.settings_path, &updated)?;
        *settings = updated.clone();
        Ok(updated)
    }

    /// The providers currently on display, in the order every surface shows them.
    fn enabled_providers(&self) -> Vec<ProviderKind> {
        self.detected_providers.lock().map(|providers| providers.clone()).unwrap_or_default()
    }

    /// Which provider clients have left usage records on this machine. These are the only
    /// providers the live reader, history parser and filesystem watcher may touch.
    fn local_providers() -> Vec<ProviderKind> {
        // A demo instance reads no provider, so nothing about this machine may decide which
        // ones it shows: it answers for every provider the seeded database describes.
        if demo::requested() {
            return ProviderKind::ALL.to_vec();
        }
        ProviderKind::ALL.into_iter().filter(|provider| provider.is_installed()).collect()
    }

    /// Whether this provider's quota is tracked: read from its client and drawn beside it.
    ///
    /// Switching it off is about the quota alone. The usage history is parsed from files
    /// that are already on disk and costs nothing to keep reading, so it carries on — what
    /// stops is starting the client to ask for a percentage nobody wants to see.
    fn quota_tracked(settings: &AppSettings, provider: ProviderKind) -> bool {
        !settings.quota_disabled_providers.iter().any(|key| key == provider.key())
    }

    /// How many columns the quick panel draws: every provider on display whose quota is
    /// tracked. Quota is what that panel is, so one switched off takes no column there and
    /// the window opens at the width of the columns that remain. The switch only reaches a
    /// provider this machine can read, exactly as `workspace_snapshot` resolves it — one
    /// whose usage arrives from another device keeps its column and says so in it.
    fn quota_column_count(&self) -> usize {
        let settings = self.settings();
        let local = Self::local_providers();
        self.enabled_providers()
            .into_iter()
            .filter(|provider| {
                !local.contains(provider) || Self::quota_tracked(&settings, *provider)
            })
            .count()
    }

    /// The providers whose live quota may be read right now.
    fn quota_providers(&self) -> Vec<ProviderKind> {
        let settings = self.settings();
        Self::local_providers()
            .into_iter()
            .filter(|provider| Self::quota_tracked(&settings, *provider))
            .collect()
    }

    /// Refreshes the display list from local clients plus every provider represented by
    /// imported usage. Remote snapshots are reloaded after each import so today's compact
    /// totals move with the history tab instead of waiting for a restart.
    async fn refresh_enabled_providers(&self) -> anyhow::Result<()> {
        let local = Self::local_providers();
        let stored = self.storage.load_usage_providers().await?;
        let enabled = ProviderKind::ALL
            .into_iter()
            .filter(|provider| local.contains(provider) || stored.contains(provider))
            .collect::<Vec<_>>();
        let mut remote = Vec::new();
        for &provider in enabled.iter().filter(|provider| !local.contains(provider)) {
            let mut snapshot = self.storage.load_snapshot(provider).await?;
            snapshot.remote_usage_only = true;
            snapshot.clear_quota();
            snapshot.resolve_derived_state();
            remote.push((provider, snapshot));
        }
        {
            let mut snapshots = self.snapshots.write().await;
            snapshots.retain(|provider, _| enabled.contains(provider));
            snapshots.extend(remote);
        }
        if let Ok(mut providers) = self.detected_providers.lock() {
            providers.clone_from(&enabled);
        }
        Ok(())
    }

    /// How long to wait before reading this provider's quota again.
    ///
    /// A signed-out provider answers the same way every time until someone signs in with
    /// its own client, so it is asked once an hour instead of starting a client process on
    /// the ordinary interval for a refusal already on display.
    async fn live_refresh_delay(&self, provider: ProviderKind) -> Duration {
        if self.read_snapshot(provider, |snapshot| snapshot.sign_in_required).await {
            providers::SIGNED_OUT_REFRESH_INTERVAL
        } else {
            provider.live_refresh_interval()
        }
    }

    async fn with_snapshot(
        &self,
        provider: ProviderKind,
        edit: impl FnOnce(&mut ProviderSnapshot),
    ) {
        let mut snapshots = self.snapshots.write().await;
        edit(snapshots.entry(provider).or_insert_with(|| ProviderSnapshot::new(provider)));
    }

    async fn read_snapshot<T>(
        &self,
        provider: ProviderKind,
        read: impl FnOnce(&ProviderSnapshot) -> T,
    ) -> T {
        if let Some(snapshot) = self.snapshots.read().await.get(&provider) {
            return read(snapshot);
        }
        read(&ProviderSnapshot::new(provider))
    }

    /// The payload every surface consumes. Derived state is resolved here so a snapshot
    /// never reaches the renderer with a status that disagrees with its own errors.
    async fn workspace_snapshot(&self) -> WorkspaceSnapshot {
        let local = Self::local_providers();
        let settings = self.settings();
        let snapshots = self.snapshots.read().await;
        let providers = self
            .enabled_providers()
            .into_iter()
            .map(|provider| {
                let mut snapshot = snapshots
                    .get(&provider)
                    .cloned()
                    .unwrap_or_else(|| ProviderSnapshot::new(provider));
                snapshot.remote_usage_only = !local.contains(&provider);
                snapshot.quota_disabled =
                    !snapshot.remote_usage_only && !Self::quota_tracked(&settings, provider);
                if snapshot.remote_usage_only || snapshot.quota_disabled {
                    snapshot.clear_quota();
                }
                snapshot.resolve_derived_state();
                snapshot
            })
            .collect();
        WorkspaceSnapshot::new(providers)
    }
}

/// Placement runs on a short loop so the widget follows taskbar changes. Repeating the
/// same failure every tick would bury every other message, so only changes are reported.
///
/// Tauri owns webview windows on its event-loop thread. In particular, the widget renderer
/// reports its size from an IPC worker and the placement timer runs on Tokio; touching the
/// native window directly from either can make a release build panic instead of leaving the
/// widget in its previous position.
fn place_taskbar_widget(app: &tauri::AppHandle) {
    static LAST_ERROR: StdMutex<Option<String>> = StdMutex::new(None);
    let error = taskbar::place_widget(app).err();
    let Ok(mut last_error) = LAST_ERROR.lock() else { return };
    if *last_error != error {
        if let Some(message) = &error {
            log::write(format!("taskbar status placement: {message}"));
        }
        *last_error = error;
    }
}

fn schedule_taskbar_widget_placement(app: &tauri::AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || place_taskbar_widget(&handle)) {
        log::write(format!("taskbar status placement could not reach the main thread: {error}"));
    }
}

fn set_taskbar_widget_visible(app: &tauri::AppHandle, visible: bool) {
    log::write(format!("taskbar status switched {}", on_off(visible)));
    let state = app.state::<Arc<AppState>>();
    if let Err(error) = state.update_settings(|settings| settings.taskbar_widget_enabled = visible)
    {
        log::write(format!("failed to save application settings: {error}"));
    }
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if visible {
            // Placement comes first: it rebuilds the window when Explorer's taskbar took it
            // with it, and showing a window that no longer exists is what ends the process.
            place_taskbar_widget(&handle);
            if let Some(widget) = handle
                .get_webview_window(&taskbar::widget_label())
                .filter(|_| taskbar::widget_is_live(&handle))
            {
                let _ = widget.show();
            }
            // A low-level hook is called on the thread that installed it, so it has to be
            // installed on the one running the message loop.
            taskbar::watch_widget_clicks();
        } else if let Some(widget) = handle
            .get_webview_window(&taskbar::widget_label())
            .filter(|_| taskbar::widget_is_live(&handle))
        {
            let _ = widget.hide();
        }
    }) {
        log::write(format!("taskbar status visibility could not reach the main thread: {error}"));
    }
}

/// Which display's taskbar the user chose to host the status widget, for [`taskbar`].
///
/// The placement loop runs before the settings dialog has ever been opened and after the
/// window it belongs to is gone, so it reads the recorded choice rather than being told.
pub(crate) fn preferred_taskbar_display(app: &tauri::AppHandle) -> Option<String> {
    app.try_state::<Arc<AppState>>()?.settings().taskbar_widget_display
}

/// The one two-second tick the application runs on.
///
/// Three things have to be noticed at about this rate, and none of them costs anything next
/// to the wake-up itself: a finished Claude Code turn the hook left behind, a Windows theme
/// change, and a taskbar that moved out from under the docked status. One ticker keeps them
/// on the same schedule instead of waking the process three times over.
///
/// `finished_turns` is off for a demo start, which has no hook and no session to report.
fn watch_the_desktop(app: tauri::AppHandle, finished_turns: bool) {
    tauri::async_runtime::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        let mut last_theme = theme::snapshot(current_preference(&app));
        loop {
            ticks.tick().await;
            if finished_turns {
                raise_finished_turns(&app);
            }
            last_theme = follow_system_theme(&app, last_theme);
            if app.state::<Arc<AppState>>().settings().taskbar_widget_enabled {
                schedule_taskbar_widget_placement(&app);
            }
        }
    });
}

/// Raises the desktop notification a finished Claude Code turn left behind.
///
/// The hook process cannot show one itself — it has no window, no event loop, and a few
/// milliseconds to live — so it writes an event and this picks it up. Polling one path is
/// what that costs; the alternative is a filesystem watcher for a file written a handful of
/// times an hour.
fn raise_finished_turns(app: &tauri::AppHandle) {
    // The title says which event this is, the same way the quota notifications do. Windows
    // already prints the application's name above it, so spending the title on
    // "QuotaStation" left every notification looking alike in the action centre.
    for event in notifications::take_pending(jiff::Timestamp::now().as_second()) {
        let body = finished_body(&event);
        match event.terminal {
            // Clicking goes back to the terminal the turn ran in. The tab inside it is the
            // user's to pick: nothing outside Windows Terminal can choose one.
            Some(target) => {
                alerts::raise_with_action(
                    app,
                    "Claude Code finished responding",
                    &body,
                    move || {
                        if !terminal::focus(target) {
                            log::write("the terminal a finished turn ran in could not be raised");
                        }
                    },
                );
            }
            None => alerts::raise(app, "Claude Code finished responding", &body),
        }
    }
}

/// Puts the resolved theme where the two things that need it can see it: the native window
/// frame, which Windows draws and CSS cannot reach, and the renderer, which draws the rest.
///
/// The taskbar widget is left out of the frame call deliberately — it has no frame, and its
/// palette follows the taskbar rather than the preference.
fn apply_theme(app: &tauri::AppHandle, preference: theme::ThemePreference) -> theme::ThemeSnapshot {
    let snapshot = theme::snapshot(preference);
    let frame = match preference {
        theme::ThemePreference::System => None,
        theme::ThemePreference::Dark => Some(tauri::Theme::Dark),
        theme::ThemePreference::Light => Some(tauri::Theme::Light),
    };
    for window in app.webview_windows().values() {
        if !window.label().starts_with("taskbar-widget") {
            let _ = window.set_theme(frame);
        }
    }
    let _ = app.emit("theme-changed", snapshot);
    snapshot
}

/// Notices a Windows theme change while QuotaStation is running, and answers with the
/// palettes now in force.
///
/// Windows announces this to windows that have not been told what theme to be, and every
/// window here has been, so the announcement never arrives. Reading two registry values is
/// cheap enough to do on the tick everything else in this application already runs on, and
/// only a change is published.
fn follow_system_theme(app: &tauri::AppHandle, last: theme::ThemeSnapshot) -> theme::ThemeSnapshot {
    let preference = current_preference(app);
    let snapshot = theme::snapshot(preference);
    if snapshot == last {
        return last;
    }
    apply_theme(app, preference);
    snapshot
}

fn current_preference(app: &tauri::AppHandle) -> theme::ThemePreference {
    app.state::<Arc<AppState>>().settings().theme
}

/// Which session finished, in the width a notification body has.
///
/// Someone with one terminal open needs neither line and reads the title alone; someone with
/// six needs to know which of them is waiting, and the project directory answers that until
/// two sessions share it. The session title is what separates those two, so it is added
/// rather than substituted — a title says what the work is, never where it is.
fn finished_body(event: &notifications::FinishedEvent) -> String {
    match (event.project.as_deref(), event.session.as_deref()) {
        (Some(project), Some(session)) => format!("{project} \u{b7} {session}"),
        (Some(project), None) => project.to_string(),
        (None, Some(session)) => session.to_string(),
        (None, None) => "A turn has ended".to_string(),
    }
}

/// Which build is running, told apart the way the machine can tell them apart: the compiler
/// knows debug from release, and an installer leaves an uninstaller beside the executable
/// while a portable copy does not.
fn build_kind() -> String {
    if cfg!(debug_assertions) {
        return "debug".to_string();
    }
    let installed = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|dir| dir.join("uninstall.exe")))
        .is_some_and(|uninstaller| uninstaller.exists());
    if installed { "release, installed".to_string() } else { "release, portable".to_string() }
}

/// Codex logs the server's rate-limit answer alongside its own token counts, which
/// reaches back further than this database and covers every stretch when QuotaStation was
/// closed. Replaying it on startup is what makes the restart history complete rather than
/// starting from whenever this feature was installed.
async fn backfill_resets(state: &Arc<AppState>) -> anyhow::Result<()> {
    for provider in AppState::local_providers() {
        let since = state.storage.reset_backfill_start(provider).await?;
        let observations = providers::read_observations(provider, since).await?;
        if observations.is_empty() {
            continue;
        }
        let scanned_at = jiff::Timestamp::now().to_string();
        state.storage.backfill_resets(provider, &observations, &scanned_at).await?;
    }
    Ok(())
}

pub fn run() {
    let mut builder = tauri::Builder::default();
    // Single instance is how a second launch reaches the running dashboard instead of
    // starting a rival tray icon. A demo is the one launch that has to stand beside the
    // real application rather than hand over to it, so it stays out of that arrangement.
    if !demo::requested() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second launch is how an already-running QuotaStation is asked for its
            // dashboard, unless that launch was itself a background one.
            if !args.iter().any(|argument| argument == autostart::BACKGROUND_ARG) {
                show_main(app);
            }
        }));
    }
    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![autostart::BACKGROUND_ARG]),
        ))
        .setup(|app| {
            log::write(format!(
                "application started, version {} ({} build {}), {}, status line {}, finished-turn hook {}",
                app.package_info().version,
                build_kind(),
                env!("QUOTASTATION_BUILD_COMMIT"),
                if autostart::requested() { "started in the background" } else { "started with a window" },
                match statusline::bridge_status().installed {
                    true => "installed",
                    false => "not installed",
                },
                on_off(notifications::installed()),
            ));
            let app_data_dir = app.path().app_data_dir()?;
            let demo = demo::requested();
            if demo {
                log::write("started as a demonstration; no provider will be read");
            }
            let database_path =
                app_data_dir.join(if demo { demo::DATABASE_FILE } else { "quotastation.db" });
            let settings_path =
                app_data_dir.join(if demo { demo::SETTINGS_FILE } else { "settings.json" });
            let settings = ensure_device_identity(&settings_path, settings::load(&settings_path));
            // `settings::load` has already dropped a name the zone database does not know.
            if let Err(error) = clock::choose(settings.time_zone.as_deref()) {
                log::write(format!("the chosen time zone could not be applied: {error:#}"));
            }
            let device_name =
                settings.device_name.clone().unwrap_or_else(settings::default_device_name);
            let storage = tauri::async_runtime::block_on(Storage::open(&database_path))
                .map_err(|error| error.to_string())?;
            if let Err(error) = tauri::async_runtime::block_on(storage.run_retention_if_due()) {
                log::write(format!("normalized data retention failed: {error:#}"));
            }
            let local_providers = AppState::local_providers();
            log::write(format!(
                "provider clients found: [{}], quota tracked for [{}]",
                local_providers.iter().map(|provider| provider.key()).collect::<Vec<_>>().join(" "),
                local_providers
                    .iter()
                    .filter(|provider| AppState::quota_tracked(&settings, **provider))
                    .map(|provider| provider.key())
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
            let mut snapshots = BTreeMap::new();
            for &provider in &local_providers {
                let snapshot = tauri::async_runtime::block_on(storage.load_snapshot(provider))
                    .unwrap_or_else(|_| ProviderSnapshot::new(provider));
                snapshots.insert(provider, snapshot);
            }
            let state = Arc::new(AppState::new(
                storage,
                settings,
                settings_path,
                snapshots,
                local_providers,
            ));
            // What the device split calls this machine, so a split reads "Workshop" rather
            // than an identifier — and follows the machine being renamed.
            if let Err(error) =
                tauri::async_runtime::block_on(state.storage.record_local_device(&device_name))
            {
                log::write(format!("this machine could not be named: {error:#}"));
            }
            if let Err(error) = tauri::async_runtime::block_on(state.refresh_enabled_providers()) {
                log::write(format!("usage providers could not be loaded: {error:#}"));
            }
            app.manage(state.clone());
            let _ = APP.set(app.handle().clone());
            build_tray(app)?;
            log::write(format!(
                "providers on display: [{}]",
                state
                    .enabled_providers()
                    .iter()
                    .map(|provider| provider.key())
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
            // The dashboard is configured hidden so a background start never flashes a
            // window on its way to the tray; every other start opens it here instead.
            if autostart::requested() {
                log::write("started in the background; the dashboard stays closed");
            } else {
                show_main(app.handle());
            }
            if !demo {
                reinstall::restore_after_reinstall(app.handle());
                autostart::refresh_logon_entry(app.handle());
            }
            apply_theme(app.handle(), state.settings().theme);
            watch_the_desktop(app.handle().clone(), !demo);
            if state.settings().taskbar_widget_enabled {
                set_taskbar_widget_visible(app.handle(), true);
            }
            // Everything below reads a provider, so a demo start does none of it: the seeded
            // readings are what it exists to show, and the first refresh would replace them
            // with this machine's real usage.
            if !demo {
                if session_watcher::start(app.handle().clone(), state.clone()).is_err() {
                    tauri::async_runtime::block_on(async {
                        let mut diagnostics = state.watcher_diagnostics.write().await;
                        diagnostics.status = "unavailable".to_string();
                        diagnostics.error = Some(
                            "Session watching is unavailable; periodic reconciliation remains active."
                                .to_string(),
                        );
                    });
                }
                let app_handle = app.handle().clone();
                let refresh_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    refresh::refresh_all(&app_handle, &refresh_state).await;
                });
                let backfill_state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = backfill_resets(&backfill_state).await {
                        log::write(format!("quota reset backfill failed: {error:#}"));
                    }
                });
            }
            let retention_storage = app.state::<Arc<AppState>>().storage.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(error) = retention_storage.run_retention_if_due().await {
                        log::write(format!("normalized data retention failed: {error:#}"));
                    }
                }
            });
            if !demo {
                // Each provider polls on its own interval: a local process tolerates a
                // frequent read, a rate-limited remote endpoint does not.
                for provider in ProviderKind::ALL {
                    let app_handle = app.handle().clone();
                    let live_state = app.state::<Arc<AppState>>().inner().clone();
                    tauri::async_runtime::spawn(async move {
                        loop {
                            tokio::time::sleep(live_state.live_refresh_delay(provider).await).await;
                            refresh::refresh_live_for_provider(&app_handle, &live_state, provider)
                                .await;
                        }
                    });
                }
                let app_handle = app.handle().clone();
                let history_state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(900));
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        refresh::refresh_history(&app_handle, &history_state).await;
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_usage_start_date,
            commands::get_usage_range,
            commands::get_usage_hours,
            commands::get_usage_window,
            commands::get_quota_history,
            commands::get_session_costs,
            commands::get_reset_history,
            commands::refresh_now,
            commands::get_diagnostics,
            commands::export_diagnostics,
            shell::reveal_export_file,
            shell::get_log_available,
            shell::reveal_log_file,
            shell::open_data_folder,
            shell::open_latest_release,
            commands::get_claude_status_line,
            commands::set_claude_status_line,
            commands::preview_claude_status_line,
            commands::get_claude_notifications,
            commands::set_claude_notifications,
            tray::open_dashboard,
            commands::set_taskbar_widget_size,
            commands::get_taskbar_displays,
            quick_panel::set_quick_panel_height,
            commands::get_theme,
            commands::get_app_settings,
            commands::set_app_settings,
            commands::get_provider_choices,
            commands::log_activity,
            shell::get_autostart,
            shell::set_autostart,
            shell::shared_folder_exists,
            shell::create_shared_folder,
            shell::create_desktop_shortcut
        ])
        .on_window_event(|window, event| {
            if window.label() == "quick-panel"
                && matches!(event, tauri::WindowEvent::Focused(false))
            {
                let state = window.state::<Arc<AppState>>();
                // A panel opened by a click on somebody else's window is told it lost focus
                // before it ever had it — the click belongs to that window, and Windows hands
                // the foreground back. Dismissing on that is dismissing the panel the click
                // just asked for, which looks like the click doing nothing at all.
                if let Ok(mut timing) = state.quick_panel.lock() {
                    if timing
                        .shown_at
                        .is_some_and(|shown_at| shown_at.elapsed() < Duration::from_millis(400))
                    {
                        return;
                    }
                    timing.focus_lost_at = Some(Instant::now());
                }
                let _ = window.hide();
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                log::write(format!("{} window closed to the tray", window.label()));
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running QuotaStation");
}
