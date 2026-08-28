//! A desktop notification when Claude Code finishes answering.
//!
//! Claude Code says nothing when a long turn completes unless the terminal it runs in is one
//! of the few its own notification channel knows how to reach, which on Windows means
//! stepping back to the window to find out whether it is still working. Claude Code does,
//! however, run a `Stop` hook the moment the main agent stops responding, so QuotaStation
//! registers itself there the same way it registers as the status line — with the user's
//! consent, alongside whatever else is already configured, and never replacing it.
//!
//! The hook process is short-lived and has no handle on the running application, so it
//! leaves a one-line event in the application data directory and exits; the application
//! picks it up and raises the notification.
//!
//! A notification that only says a turn ended is no use to someone running Claude Code in
//! several terminals at once, so the event names the session: the project directory's last
//! segment, and the session title Claude Code shows for it. The hook is handed neither —
//! its payload carries an identifier — so the status-line bridge, which is handed both on
//! every render, keeps a small local register of what each session id is called and the
//! hook looks its own session up there. The register and the events stay inside
//! QuotaStation's application data directory; nothing else from the conversation is
//! written down, and nothing leaves the machine.

use std::{env, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::statusline::{app_data_dir, load_settings_with_source, settings_path, write_settings};

/// The argument that turns this executable into the Stop hook. As with the status-line
/// bridge, it is specific enough that a stray argument can never be mistaken for it.
pub const HOOK_ARG: &str = "--claude-notify";

/// One directory holding one file per finished turn. Two sessions that end together are two
/// notifications, because the whole point of naming the session is that they are not
/// interchangeable — a single shared file would let the later one erase the earlier.
const EVENT_DIR: &str = "claude-notifications";

/// Where the status-line bridge records what each session id is called.
const REGISTER_FILE: &str = "claude-sessions.json";

/// An event older than this belongs to a turn that finished while the application was
/// closed. Announcing it now would be a notification about something long over.
const MAX_EVENT_AGE_SECS: i64 = 60;

/// A session unheard from for this long has ended; keeping its title serves nothing.
const MAX_REGISTER_AGE_SECS: i64 = 24 * 60 * 60;

/// Sessions worth remembering at once. Well past the number of terminals anyone keeps open,
/// and small enough that the register is rewritten without thought.
const REGISTER_LIMIT: usize = 32;

/// The status-line bridge is a new process on every render, so a process-local mutex cannot
/// protect the shared register. A named Windows mutex is released automatically if one of
/// those short-lived processes exits while holding it.
#[cfg(windows)]
struct SessionRegisterLock(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl SessionRegisterLock {
    fn acquire() -> Option<Self> {
        use windows::{
            Win32::{
                Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0},
                System::Threading::{CreateMutexW, WaitForSingleObject},
            },
            core::w,
        };

        let handle =
            unsafe { CreateMutexW(None, false, w!("Local\\QuotaStationClaudeSessionRegister")) }
                .ok()?;
        let wait = unsafe { WaitForSingleObject(handle, 1_000) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Some(Self(handle))
        } else {
            let _ = unsafe { CloseHandle(handle) };
            None
        }
    }
}

#[cfg(windows)]
impl Drop for SessionRegisterLock {
    fn drop(&mut self) {
        use windows::Win32::{Foundation::CloseHandle, System::Threading::ReleaseMutex};

        let _ = unsafe { ReleaseMutex(self.0) };
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(not(windows))]
struct SessionRegisterLock {
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(not(windows))]
impl SessionRegisterLock {
    fn acquire() -> Option<Self> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().ok().map(|guard| Self { _guard: guard })
    }
}

/// What Claude Code hands the Stop hook. Two fields are read: which session ended, and
/// where it was running. The payload also carries the assistant's final message, and that
/// is conversation content this has no reason to write to disk.
#[derive(Deserialize)]
struct StopHookInput {
    cwd: Option<String>,
    session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FinishedEvent {
    pub observed_at: i64,
    /// The project directory's last segment, which is how the user tells one terminal from
    /// another. The full path stays where it was.
    pub project: Option<String>,
    /// What Claude Code calls this session, when the bridge has seen it. Two sessions in one
    /// project are otherwise indistinguishable, which is the case the title exists for.
    pub session: Option<String>,
    /// The terminal window the turn was running in, so clicking the notification can go
    /// back to it. Absent whenever the hook could not find one.
    #[serde(default)]
    pub terminal: Option<crate::terminal::TerminalTarget>,
}

/// What the status-line bridge knows about a session, for the hook to read back.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct SessionRecord {
    id: String,
    name: Option<String>,
    project: Option<String>,
    updated_at: i64,
}

fn event_dir() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(EVENT_DIR))
}

fn register_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(REGISTER_FILE))
}

/// Records what a session is called, from the status-line payload that names it.
///
/// The bridge runs on every render, so this is written far more often than it is read. It
/// is one small file rewritten in place rather than a growing log: the register describes
/// the sessions running now, and one that has stopped rendering falls out of it.
pub fn record_session(id: &str, name: Option<&str>, project: Option<&str>, now: i64) {
    let Some(_lock) = SessionRegisterLock::acquire() else { return };
    let mut register = load_register();
    // A title that is already right does not need the file rewritten three times a second,
    // so an unchanged record is only refreshed often enough to stay clear of its own expiry.
    if register.iter().any(|record| {
        record.id == id
            && record.name.as_deref() == name
            && record.project.as_deref() == project
            && now - record.updated_at < 60
    }) {
        return;
    }
    register.retain(|record| record.id != id && now - record.updated_at <= MAX_REGISTER_AGE_SECS);
    register.insert(
        0,
        SessionRecord {
            id: id.to_string(),
            name: name.map(str::to_string),
            project: project.map(str::to_string),
            updated_at: now,
        },
    );
    register.truncate(REGISTER_LIMIT);
    if let Some(path) = register_path()
        && let Ok(encoded) = serde_json::to_string(&register)
    {
        let _ = publish(&path, &encoded);
    }
}

fn load_register() -> Vec<SessionRecord> {
    let Some(path) = register_path() else { return Vec::new() };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Runs as Claude Code's Stop hook when this executable was started with [`HOOK_ARG`], and
/// reports whether it did.
///
/// Claude Code waits for this process, so it does the least it can: read, write one small
/// file, exit. It prints nothing — a Stop hook's stdout is not shown — and it fails silently,
/// because a missed notification must never become an error inside someone else's turn.
pub fn run_hook_if_requested() -> bool {
    if !env::args_os().any(|argument| argument == HOOK_ARG) {
        return false;
    }
    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);
    let input = serde_json::from_str::<StopHookInput>(&payload).ok();
    let session_id = input.as_ref().and_then(|input| input.session_id.clone());
    let known = session_id
        .as_deref()
        .and_then(|id| load_register().into_iter().find(|record| record.id == id));
    let event = FinishedEvent {
        observed_at: jiff::Timestamp::now().as_second(),
        // The hook's own directory is the authority on where the turn ran; the register
        // only fills in for a payload that omitted it.
        project: input
            .and_then(|input| input.cwd)
            .and_then(|cwd| {
                std::path::Path::new(&cwd)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .or_else(|| known.as_ref().and_then(|record| record.project.clone())),
        session: known.and_then(|record| record.name),
        // Found here rather than in the application, because only this process is a child
        // of the terminal in question.
        terminal: crate::terminal::owning_window(),
    };
    if let Some(dir) = event_dir() {
        // One file per session, so sessions that finish together each announce themselves.
        // A session with no id has only this process to be told apart by.
        let name = session_id.as_deref().map_or_else(process_stem, file_stem);
        let _ = write_event(&dir.join(format!("{name}.json")), &event);
    }
    true
}

/// A session id reduced to what is safe in a file name. Claude Code's ids are hyphenated
/// hex, so in practice nothing is dropped; the id is only ever used to keep two events
/// apart, never read back.
fn file_stem(id: &str) -> String {
    let stem: String = id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(64)
        .collect();
    if stem.is_empty() { process_stem() } else { stem }
}

fn process_stem() -> String {
    format!("pid-{}", std::process::id())
}

fn write_event(path: &std::path::Path, event: &FinishedEvent) -> std::io::Result<()> {
    publish(path, &serde_json::to_string(event).unwrap_or_default())
}

/// Writes a file the way every reader here expects to find it: whole, or not at all.
/// Several sessions write at once and the application may be reading, so the content is
/// staged under this process's own name and moved into place.
fn publish(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staging, content)?;
    std::fs::rename(&staging, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })
}

/// Every event the hooks left behind, removed as they are read.
///
/// Removing them is what makes a finished turn announce itself once. A stale event — left
/// by a turn that ended while the application was closed — is dropped rather than announced,
/// but it is still taken away, or it would be reconsidered every two seconds forever.
pub fn take_pending(now: i64) -> Vec<FinishedEvent> {
    let Some(dir) = event_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut events = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok();
        let _ = std::fs::remove_file(&path);
        let Some(event) = content.and_then(|content| serde_json::from_str(&content).ok()) else {
            continue;
        };
        let event: FinishedEvent = event;
        if event.observed_at <= now + 5 && now - event.observed_at <= MAX_EVENT_AGE_SECS {
            events.push(event);
        }
    }
    events.sort_by_key(|event| event.observed_at);
    events
}

/// Whether Claude Code is configured to tell QuotaStation that a turn finished.
pub fn installed() -> bool {
    settings_path()
        .ok()
        .and_then(|path| load_settings_with_source(&path).ok())
        .is_some_and(|(settings, _)| !hook_positions(&settings).is_empty())
}

/// The indices of the Stop entries that belong to QuotaStation, so everything else in the
/// array is left exactly where it is.
fn hook_positions(settings: &serde_json::Value) -> Vec<usize> {
    let Some(entries) = settings.get("hooks").and_then(|hooks| hooks.get("Stop")) else {
        return Vec::new();
    };
    let Some(entries) = entries.as_array() else { return Vec::new() };
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .get("hooks")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|commands| commands.iter().any(is_notification_handler))
        })
        .map(|(index, _)| index)
        .collect()
}

fn is_notification_handler(handler: &serde_json::Value) -> bool {
    handler
        .get("command")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|command| command.contains(HOOK_ARG))
}

pub fn hook_command() -> Result<String> {
    let executable = env::current_exe().context("locate the QuotaStation executable")?;
    Ok(format!("\"{}\" {HOOK_ARG}", executable.display()))
}

pub fn install() -> Result<()> {
    install_into(&settings_path()?, &hook_command()?)
}

pub fn remove() -> Result<()> {
    remove_from(&settings_path()?)
}

fn install_into(path: &std::path::Path, command: &str) -> Result<()> {
    let (mut settings, source) = load_settings_with_source(path)?;
    remove_positions(&mut settings);
    let entry = serde_json::json!({
        "matcher": "",
        "hooks": [{ "type": "command", "command": command }],
    });
    let object =
        settings.as_object_mut().context("the Claude Code settings are not a JSON object")?;
    let hooks =
        object.entry("hooks").or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let hooks = hooks.as_object_mut().context("the Claude Code hooks are not a JSON object")?;
    let stop = hooks.entry("Stop").or_insert_with(|| serde_json::Value::Array(Vec::new()));
    stop.as_array_mut().context("the Claude Code Stop hooks are not a JSON array")?.push(entry);
    write_settings(path, &settings, source.as_deref())
}

fn remove_from(path: &std::path::Path) -> Result<()> {
    let (mut settings, source) = load_settings_with_source(path)?;
    if !remove_positions(&mut settings) {
        return Ok(());
    }
    write_settings(path, &settings, source.as_deref())
}

/// Drops QuotaStation's own Stop handlers, and reports whether there were any. A matcher
/// group can own several handlers, so the group survives until the last one is removed.
/// An array left empty is removed with its key: a hook that is not installed should leave
/// no trace of having been.
fn remove_positions(settings: &mut serde_json::Value) -> bool {
    let Some(hooks) = settings.get_mut("hooks").and_then(serde_json::Value::as_object_mut) else {
        return false;
    };
    let mut removed = false;
    if let Some(stop) = hooks.get_mut("Stop").and_then(serde_json::Value::as_array_mut) {
        for index in (0..stop.len()).rev() {
            let mut remove_group = false;
            if let Some(handlers) =
                stop[index].get_mut("hooks").and_then(serde_json::Value::as_array_mut)
            {
                let before = handlers.len();
                handlers.retain(|handler| !is_notification_handler(handler));
                if handlers.len() != before {
                    removed = true;
                    remove_group = handlers.is_empty();
                }
            }
            if remove_group {
                stop.remove(index);
            }
        }
        if stop.is_empty() {
            hooks.remove("Stop");
        }
    }
    if hooks.is_empty()
        && let Some(object) = settings.as_object_mut()
    {
        object.remove("hooks");
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("quotastation-notify-{name}.json"));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn installing_leaves_every_other_stop_hook_where_it_was() {
        let path = scratch("install");
        std::fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"mine.exe"}]}],
                "PreToolUse":[{"matcher":"Bash"}]}}"#,
        )
        .unwrap();
        install_into(&path, "\"q.exe\" --claude-notify").expect("install the hook");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "mine.exe");
        assert_eq!(stop[1]["hooks"][0]["command"], "\"q.exe\" --claude-notify");

        remove_from(&path).expect("remove the hook");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "only ours is taken out again");
        assert_eq!(stop[0]["hooks"][0]["command"], "mine.exe");
        assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn removing_the_only_hook_leaves_no_trace_of_it() {
        let path = scratch("only");
        install_into(&path, "\"q.exe\" --claude-notify").expect("install the hook");
        assert!(installed_in(&path));
        remove_from(&path).expect("remove the hook");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(settings.get("hooks").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn installing_twice_registers_one_hook() {
        let path = scratch("twice");
        install_into(&path, "\"q.exe\" --claude-notify").expect("install the hook");
        install_into(&path, "\"q.exe\" --claude-notify").expect("install it again");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn removing_preserves_another_handler_in_the_same_matcher_group() {
        let path = scratch("shared-group");
        std::fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"matcher":"","hooks":[
                {"type":"command","command":"mine.exe"},
                {"type":"command","command":"\"q.exe\" --claude-notify"}
            ]}]}}"#,
        )
        .unwrap();

        remove_from(&path).expect("remove only the QuotaStation handler");
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "the shared matcher group survives");
        let handlers = stop[0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0]["command"], "mine.exe");
        let _ = std::fs::remove_file(&path);
    }

    fn installed_in(path: &std::path::Path) -> bool {
        let (settings, _) = load_settings_with_source(path).unwrap();
        !hook_positions(&settings).is_empty()
    }

    /// The hook writes the event and the application reads it back, so every field has to
    /// survive the file between them.
    #[test]
    fn an_event_survives_the_file_the_hook_leaves_it_in() {
        let path = scratch("event");
        let written = FinishedEvent {
            observed_at: 1_800_000_000,
            project: Some("Q".into()),
            session: Some("Dark theme".into()),
            terminal: Some(crate::terminal::TerminalTarget { window: 1234, process_id: 5678 }),
        };
        write_event(&path, &written).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(serde_json::from_str::<FinishedEvent>(&content).unwrap(), written);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_session_id_becomes_one_event_file_per_session() {
        let first = file_stem("2f1c4b8a-0e5d-4a19-9b7c-3d6e8f0a1b2c");
        assert_eq!(first, "2f1c4b8a-0e5d-4a19-9b7c-3d6e8f0a1b2c");
        assert_ne!(first, file_stem("9a0b1c2d-3e4f-5061-7283-94a5b6c7d8e9"));
        assert!(!file_stem("../../escape").contains('.'), "nothing walks out of the directory");
        assert!(file_stem("").starts_with("pid-"), "an id that survives nothing still names one");
    }
}
