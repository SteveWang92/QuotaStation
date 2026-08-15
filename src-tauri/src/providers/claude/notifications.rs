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
//! picks it up and raises the notification. Nothing from the conversation is written down:
//! the event carries the project directory's last segment and the time, which is all the
//! notification says.

use std::{env, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::statusline::{app_data_dir, load_settings_with_source, settings_path, write_settings};

/// The argument that turns this executable into the Stop hook. As with the status-line
/// bridge, it is specific enough that a stray argument can never be mistaken for it.
pub const HOOK_ARG: &str = "--claude-notify";

const EVENT_FILE: &str = "claude-notification.json";

/// An event older than this belongs to a turn that finished while the application was
/// closed. Announcing it now would be a notification about something long over.
const MAX_EVENT_AGE_SECS: i64 = 60;

/// What Claude Code hands the Stop hook. Only the directory is read: the payload also
/// carries the assistant's final message, and that is conversation content this has no
/// reason to write to disk.
#[derive(Deserialize)]
struct StopHookInput {
    cwd: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct FinishedEvent {
    pub observed_at: i64,
    /// The project directory's last segment, which is how the user tells one terminal from
    /// another. The full path stays where it was.
    pub project: Option<String>,
}

fn event_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(EVENT_FILE))
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
    let project = serde_json::from_str::<StopHookInput>(&payload)
        .ok()
        .and_then(|input| input.cwd)
        .and_then(|cwd| {
            std::path::Path::new(&cwd)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
    let event = FinishedEvent { observed_at: jiff::Timestamp::now().as_second(), project };
    if let Some(path) = event_path() {
        let _ = write_event(&path, &event);
    }
    true
}

fn write_event(path: &std::path::Path, event: &FinishedEvent) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Several sessions can finish at once, and the application may be reading: published by
    // rename so a reader never sees half an event.
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staging, serde_json::to_string(event).unwrap_or_default())?;
    std::fs::rename(&staging, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })
}

/// The event a hook left behind, removed as it is read.
///
/// Removing it is what makes the same finished turn announce itself once: two sessions
/// finishing together produce one notification rather than a pair, which is the right
/// trade for a signal that only means "go and look".
pub fn take_pending(now: i64) -> Option<FinishedEvent> {
    let path = event_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let event: FinishedEvent = serde_json::from_str(&content).ok()?;
    (event.observed_at <= now + 5 && now - event.observed_at <= MAX_EVENT_AGE_SECS).then_some(event)
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
                .is_some_and(|commands| {
                    commands.iter().any(|command| {
                        command
                            .get("command")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|command| command.contains(HOOK_ARG))
                    })
                })
        })
        .map(|(index, _)| index)
        .collect()
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
    let object = settings
        .as_object_mut()
        .context("the Claude Code settings are not a JSON object")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
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

/// Drops QuotaStation's own Stop entries, and reports whether there were any. An array left
/// empty is removed with its key: a hook that is not installed should leave no trace of
/// having been.
fn remove_positions(settings: &mut serde_json::Value) -> bool {
    let positions = hook_positions(settings);
    if positions.is_empty() {
        return false;
    }
    let Some(hooks) = settings.get_mut("hooks").and_then(serde_json::Value::as_object_mut) else {
        return false;
    };
    if let Some(stop) = hooks.get_mut("Stop").and_then(serde_json::Value::as_array_mut) {
        for index in positions.into_iter().rev() {
            stop.remove(index);
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
    true
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

    fn installed_in(path: &std::path::Path) -> bool {
        let (settings, _) = load_settings_with_source(path).unwrap();
        !hook_positions(&settings).is_empty()
    }

    #[test]
    fn an_event_left_behind_while_the_application_was_closed_is_not_announced() {
        let now = 1_800_000_000;
        let path = scratch("event");
        write_event(&path, &FinishedEvent { observed_at: now, project: Some("Q".into()) }).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let event: FinishedEvent = serde_json::from_str(&content).unwrap();
        assert_eq!(event.project.as_deref(), Some("Q"));
        assert!(now - event.observed_at <= MAX_EVENT_AGE_SECS);
        assert!(now + 3_600 - event.observed_at > MAX_EVENT_AGE_SECS, "an hour later it is stale");
        let _ = std::fs::remove_file(&path);
    }
}
