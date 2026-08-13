//! Which Claude Code sessions are running right now, and where.
//!
//! Claude Code records one small file per live session under `~/.claude/sessions`, naming
//! the host it was started from, and removes it when the session ends. QuotaStation reads
//! one field of it — the entry point — because that field decides whether the status-line
//! bridge can ever produce a reading: Claude Code only renders a status line in a terminal,
//! so a session hosted by the desktop application never runs the configured command. With
//! that known, an empty quota card can say why it is empty instead of looking broken.
//!
//! Nothing else in those files is read. They also carry the session's working directory and
//! identifiers, which QuotaStation does not collect.

use std::path::Path;

use super::claude_home;

/// Entry points that host Claude Code inside their own interface rather than a terminal.
/// A host that is not listed is assumed to be a terminal, which is the safe direction: it
/// only means the interface stays quiet rather than explaining something that is not true.
const NON_TERMINAL_ENTRYPOINTS: [&str; 1] = ["claude-desktop"];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LiveSessions {
    pub total: usize,
    /// Sessions running in a terminal, which are the only ones that render a status line.
    pub terminal: usize,
}

impl LiveSessions {
    /// Whether Claude Code is running, but only where no status line is ever rendered.
    pub fn desktop_only(self) -> bool {
        self.total > 0 && self.terminal == 0
    }
}

pub fn live_sessions() -> LiveSessions {
    claude_home().map(|home| count_in(&home.join("sessions"))).unwrap_or_default()
}

fn count_in(directory: &Path) -> LiveSessions {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return LiveSessions::default();
    };
    let mut sessions = LiveSessions::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(entrypoint) = read_entrypoint(&path) else { continue };
        sessions.total += 1;
        if !NON_TERMINAL_ENTRYPOINTS.contains(&entrypoint.as_str()) {
            sessions.terminal += 1;
        }
    }
    sessions
}

fn read_entrypoint(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value.get("entrypoint")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("quotastation-{name}-sessions"));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create session fixture");
        for (file, content) in files {
            std::fs::write(directory.join(file), content).expect("write session fixture");
        }
        directory
    }

    #[test]
    fn a_terminal_session_is_one_that_can_render_a_status_line() {
        let directory = fixture(
            "mixed",
            &[
                ("1.json", r#"{"entrypoint":"claude-desktop"}"#),
                ("2.json", r#"{"entrypoint":"cli"}"#),
            ],
        );
        let sessions = count_in(&directory);
        assert_eq!(sessions.total, 2);
        assert_eq!(sessions.terminal, 1);
        assert!(!sessions.desktop_only());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn sessions_hosted_only_by_the_desktop_application_are_recognised() {
        let directory = fixture(
            "desktop",
            &[
                ("1.json", r#"{"entrypoint":"claude-desktop"}"#),
                ("2.json", r#"{"entrypoint":"claude-desktop"}"#),
                // Neither a stray file nor an unreadable one is a session.
                ("notes.txt", "ignored"),
                ("3.json", "{ broken"),
            ],
        );
        let sessions = count_in(&directory);
        assert_eq!(sessions.total, 2);
        assert!(sessions.desktop_only());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn nothing_running_is_not_a_desktop_only_installation() {
        let directory = fixture("empty", &[]);
        assert!(!count_in(&directory).desktop_only());
        assert_eq!(count_in(&directory.join("absent")), LiveSessions::default());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
