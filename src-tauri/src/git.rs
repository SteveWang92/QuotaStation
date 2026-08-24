//! What the working tree looks like beside the branch name, for the status line.
//!
//! The branch alone answers where the work is going, not whether any of it has been saved.
//! Two counts finish the sentence: how many paths differ from the last commit, and how far
//! the branch stands from its upstream. Both are read for the directory Claude Code is
//! running in, and neither ever leaves the machine.
//!
//! The branch is read straight from `.git/HEAD` because one file read costs nothing. The
//! two counts cannot be had that cheaply — they need the index compared against the working
//! tree and the commit graph walked — so they come from `git` itself, and a short-lived
//! cache keeps the client from paying for a process on every render of a streaming turn.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

/// Where the cached counts live, inside QuotaStation's own application data directory.
const CACHE_FILE: &str = "git-status.json";

/// How long a reading stands before `git` is asked again. Claude Code re-renders the status
/// line several times a second while a turn streams, and a count that is a few seconds old
/// is indistinguishable from a current one at the moment it is glanced at.
const CACHE_TTL_SECS: i64 = 3;

/// Repositories worth remembering between renders. Enough for every project open at once,
/// small enough that the file is rewritten without thought.
const CACHE_LIMIT: usize = 16;

/// Windows would otherwise flash a console window for the `git` child process.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How far the working tree has drifted from the last commit and from the remote.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkTreeStatus {
    /// Paths that differ from `HEAD`, untracked files included: the number of things that
    /// would be lost by walking away from this checkout.
    pub changed: usize,
    /// Commits this branch has that its upstream does not, and the other way round. Both
    /// stay zero when the branch tracks nothing, which the counts alone cannot express.
    pub ahead: u32,
    pub behind: u32,
    pub tracked: bool,
}

/// The repository containing `start`, found by walking up from it.
///
/// A worktree and a submodule leave a `gitdir:` pointer where the directory would be; the
/// pointer names the administrative directory, and the directory holding it is still the
/// root of the checkout, which is what `git` has to be pointed at.
pub fn repository_root(start: &Path) -> Option<PathBuf> {
    let mut directory = Some(start);
    while let Some(current) = directory {
        let git = current.join(".git");
        if git.is_dir() || git.is_file() {
            return Some(current.to_path_buf());
        }
        directory = current.parent();
    }
    None
}

/// The checked-out branch, read straight from `.git/HEAD`.
///
/// Claude Code renders the status line on every turn, so this must not spawn a process: a
/// `git` invocation for a name already written down is a cost the client would pay for a
/// monitor's convenience. One short file read is not. A detached head names no branch and
/// reports none.
pub fn branch_at(root: &Path) -> Option<String> {
    let git = root.join(".git");
    let head = if git.is_dir() {
        git.join("HEAD")
    } else {
        let pointer = std::fs::read_to_string(&git).ok()?;
        let admin = PathBuf::from(pointer.trim().strip_prefix("gitdir:")?.trim());
        let admin = if admin.is_absolute() { admin } else { root.join(admin) };
        admin.join("HEAD")
    };
    std::fs::read_to_string(head).ok()?.trim().strip_prefix("ref: refs/heads/").map(str::to_string)
}

/// The two counts for a repository, from the cache when it is current and from `git`
/// otherwise. Anything that goes wrong — no `git` on the path, a repository mid-rebase, a
/// checkout too large to answer in time — costs the counts and nothing else.
pub fn work_tree_status(root: &Path, now: i64) -> Option<WorkTreeStatus> {
    let key = root.to_string_lossy().into_owned();
    let mut cache = load_cache();
    if let Some(entry) = cache.iter().find(|entry| entry.root == key)
        && (0..=CACHE_TTL_SECS).contains(&(now - entry.observed_at))
    {
        return Some(entry.status);
    }
    let status = read_status(root)?;
    cache.retain(|entry| entry.root != key);
    cache.insert(0, CacheEntry { root: key, observed_at: now, status });
    cache.truncate(CACHE_LIMIT);
    store_cache(&cache);
    Some(status)
}

/// One `git status` in the machine-readable form, which reports the working tree and the
/// distance from the upstream in a single pass. `--no-optional-locks` keeps a status line
/// from taking the index lock out from under the person actually using the repository.
fn read_status(root: &Path) -> Option<WorkTreeStatus> {
    let mut command = Command::new("git");
    command
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_status(&String::from_utf8_lossy(&output.stdout)))
}

/// Reads the porcelain v2 report. Entry lines are counted rather than interpreted: what
/// kind of change a path carries is the repository's business, and the status line has room
/// for how many there are.
fn parse_status(output: &str) -> WorkTreeStatus {
    let mut status = WorkTreeStatus::default();
    for line in output.lines() {
        match line.split_once(' ') {
            // `# branch.ab +1 -2`, present only when the branch tracks something.
            Some(("#", rest)) => {
                let Some(counts) = rest.strip_prefix("branch.ab ") else { continue };
                status.tracked = true;
                for count in counts.split_whitespace() {
                    let (sign, value) = count.split_at(1);
                    let Ok(value) = value.parse::<u32>() else { continue };
                    match sign {
                        "+" => status.ahead = value,
                        "-" => status.behind = value,
                        _ => {}
                    }
                }
            }
            // Changed, renamed, unmerged and untracked entries each describe one path.
            Some(("1" | "2" | "u" | "?", _)) => status.changed += 1,
            _ => {}
        }
    }
    status
}

/// One repository's counts and when they were read.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct CacheEntry {
    root: String,
    observed_at: i64,
    status: WorkTreeStatus,
}

fn cache_path() -> Option<PathBuf> {
    crate::providers::claude::statusline::app_data_dir().map(|dir| dir.join(CACHE_FILE))
}

fn load_cache() -> Vec<CacheEntry> {
    let Some(path) = cache_path() else { return Vec::new() };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Best effort throughout: a cache that cannot be written costs a process on the next
/// render, which is not worth reporting to anyone. Published by rename, because several
/// Claude Code sessions render at once.
fn store_cache(cache: &[CacheEntry]) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    let Ok(encoded) = serde_json::to_string(cache) else { return };
    if std::fs::write(&staging, encoded).is_ok() && std::fs::rename(&staging, &path).is_err() {
        let _ = std::fs::remove_file(&staging);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_is_read_as_paths_changed_and_commits_apart() {
        let status = parse_status(
            "# branch.oid abc123\n\
             # branch.head dev\n\
             # branch.upstream origin/dev\n\
             # branch.ab +2 -3\n\
             1 .M N... 100644 100644 100644 aaa bbb src/lib.rs\n\
             2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\told.rs\n\
             u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.rs\n\
             ? notes.local.md\n",
        );
        assert_eq!(status, WorkTreeStatus { changed: 4, ahead: 2, behind: 3, tracked: true });
    }

    #[test]
    fn a_branch_that_tracks_nothing_reports_no_distance() {
        let status = parse_status("# branch.oid abc123\n# branch.head feat/thing\n");
        assert_eq!(status, WorkTreeStatus::default());
        assert!(!status.tracked);
    }

    #[test]
    fn a_clean_checkout_counts_nothing() {
        let status = parse_status("# branch.head main\n# branch.ab +0 -0\n");
        assert_eq!(status.changed, 0);
        assert!(status.tracked);
    }

    #[test]
    fn a_relative_gitdir_pointer_is_resolved_from_the_checkout_root() {
        let root = std::env::temp_dir().join(format!(
            "quotastation-git-pointer-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond()
        ));
        std::fs::create_dir_all(root.join("admin")).unwrap();
        std::fs::write(root.join(".git"), "gitdir: admin\n").unwrap();
        std::fs::write(root.join("admin/HEAD"), "ref: refs/heads/dev\n").unwrap();

        assert_eq!(branch_at(&root).as_deref(), Some("dev"));

        std::fs::remove_dir_all(root).unwrap();
    }
}
