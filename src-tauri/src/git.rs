//! What the working tree looks like beside the branch name, for the status line.
//!
//! The branch alone answers where the work is going, not whether any of it has been saved.
//! Two counts finish the sentence: how many paths differ from the last commit, and how far
//! the branch stands from its upstream. Both are read for the directory Claude Code is
//! running in, and neither ever leaves the machine.
//!
//! The branch is read straight from `.git/HEAD` because one file read costs nothing, and so
//! are the stash and an operation left in progress. The two counts cannot be had that
//! cheaply — they need the index compared against the working tree and the commit graph
//! walked — so they come from `git` itself, as do the last commit's time and its distance
//! from the newest tag. A short-lived cache keeps the client from paying for a process on
//! every render of a streaming turn, and each process runs only when the line shows what it
//! answers.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
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

/// The status line is rendered synchronously by Claude Code, so Git may never hold it up.
/// A worktree on a disconnected share, or one behind a stale `index.lock`, takes as long as
/// it takes; the cache does not help, because it is the uncached call that stalls.
const STATUS_TIMEOUT: Duration = Duration::from_secs(1);

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

/// The checked-out commit: when it was made, and how it stands against the newest tag.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct HeadCommit {
    pub committed_at: i64,
    /// `git describe --tags`: `v1.1.0-6-gde36776`, or the tag alone on a tagged commit.
    /// Unset in a repository with no tags.
    pub describe: Option<String>,
}

/// Which of the `git` readings the status line is going to show.
#[derive(Clone, Copy)]
pub struct Wanted {
    pub status: bool,
    pub head: bool,
}

/// What was asked for and could be read.
#[derive(Default)]
pub struct Reading {
    pub status: Option<WorkTreeStatus>,
    pub head: Option<HeadCommit>,
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
    std::fs::read_to_string(admin_dir(root)?.join("HEAD"))
        .ok()?
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
}

/// Where the checkout's own administrative files are: `.git` itself, or the directory a
/// worktree's or submodule's `gitdir:` pointer names.
fn admin_dir(root: &Path) -> Option<PathBuf> {
    let git = root.join(".git");
    if git.is_dir() {
        return Some(git);
    }
    let pointer = std::fs::read_to_string(&git).ok()?;
    let admin = PathBuf::from(pointer.trim().strip_prefix("gitdir:")?.trim());
    Some(if admin.is_absolute() { admin } else { root.join(admin) })
}

/// The directory every worktree of the repository shares, which is where the stash lives.
fn common_dir(admin: &Path) -> PathBuf {
    match std::fs::read_to_string(admin.join("commondir")) {
        Ok(pointer) => {
            let common = PathBuf::from(pointer.trim());
            if common.is_absolute() { common } else { admin.join(common) }
        }
        Err(_) => admin.to_path_buf(),
    }
}

/// How many entries the stash holds, from its reflog: one line per entry.
pub fn stash_count(root: &Path) -> Option<usize> {
    let log =
        std::fs::read_to_string(common_dir(&admin_dir(root)?).join("logs/refs/stash")).ok()?;
    Some(log.lines().filter(|line| !line.is_empty()).count())
}

/// A rebase, merge, cherry-pick, revert or bisect left in progress, named the way `git`'s
/// own prompt names it, with a rebase's step when `git` recorded one.
pub fn operation(root: &Path) -> Option<String> {
    let admin = admin_dir(root)?;
    let number =
        |path: PathBuf| -> Option<u32> { std::fs::read_to_string(path).ok()?.trim().parse().ok() };
    for (directory, step, last) in
        [("rebase-merge", "msgnum", "end"), ("rebase-apply", "next", "last")]
    {
        let directory = admin.join(directory);
        if directory.is_dir() {
            return Some(match (number(directory.join(step)), number(directory.join(last))) {
                (Some(step), Some(last)) => format!("REBASE {step}/{last}"),
                _ => "REBASE".to_string(),
            });
        }
    }
    [
        ("MERGE_HEAD", "MERGE"),
        ("CHERRY_PICK_HEAD", "CHERRY-PICK"),
        ("REVERT_HEAD", "REVERT"),
        ("BISECT_LOG", "BISECT"),
    ]
    .into_iter()
    .find(|(file, _)| admin.join(file).exists())
    .map(|(_, name)| name.to_string())
}

/// The readings asked for, from the cache when it is current and from `git` otherwise.
/// Anything that goes wrong — no `git` on the path, a checkout too large to answer in time —
/// costs that reading and nothing else.
pub fn read(root: &Path, now: i64, wanted: Wanted) -> Reading {
    if !wanted.status && !wanted.head {
        return Reading::default();
    }
    let key = root.to_string_lossy().into_owned();
    let mut cache = load_cache();
    let mut entry = cache
        .iter()
        .find(|entry| {
            entry.root == key && (0..=CACHE_TTL_SECS).contains(&(now - entry.observed_at))
        })
        .cloned()
        .unwrap_or_else(|| CacheEntry {
            root: key.clone(),
            observed_at: now,
            status: None,
            head: None,
        });
    let mut read_now = false;
    if wanted.status && entry.status.is_none() {
        entry.status = read_status(root);
        read_now = true;
    }
    if wanted.head && entry.head.is_none() {
        entry.head = read_head(root);
        read_now = true;
    }
    if read_now {
        cache.retain(|cached| cached.root != key);
        cache.insert(0, entry.clone());
        cache.truncate(CACHE_LIMIT);
        store_cache(&cache);
    }
    Reading {
        status: entry.status.filter(|_| wanted.status),
        head: entry.head.filter(|_| wanted.head),
    }
}

/// One `git status` in the machine-readable form, which reports the working tree and the
/// distance from the upstream in a single pass. `--no-optional-locks` keeps a status line
/// from taking the index lock out from under the person actually using the repository.
fn read_status(root: &Path) -> Option<WorkTreeStatus> {
    run_git(
        root,
        &["--no-optional-locks", "status", "--porcelain=v2", "--branch", "--untracked-files=all"],
    )
    .map(|output| parse_status(&output))
}

/// The last commit's time and its description against the newest tag, in one process.
fn read_head(root: &Path) -> Option<HeadCommit> {
    run_git(root, &["--no-optional-locks", "log", "-1", "--format=%ct%x00%(describe:tags)"])
        .and_then(|output| parse_head(&output))
}

fn parse_head(output: &str) -> Option<HeadCommit> {
    let (time, describe) = output.trim_end().split_once('\0')?;
    Some(HeadCommit {
        committed_at: time.parse().ok()?,
        describe: Some(describe.trim()).filter(|describe| !describe.is_empty()).map(str::to_string),
    })
}

/// What `git` printed, or nothing if it failed or outran [`STATUS_TIMEOUT`].
fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.args(args).current_dir(root).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = sender.send(stdout.read_to_end(&mut bytes).ok().map(|_| bytes));
    });
    let deadline = Instant::now() + STATUS_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let output = receiver.recv_timeout(Duration::from_millis(100)).ok()??;
    Some(String::from_utf8_lossy(&output).into_owned())
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

/// One repository's readings and when they were taken. A reading nobody has asked for yet
/// is absent rather than stale.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct CacheEntry {
    root: String,
    observed_at: i64,
    status: Option<WorkTreeStatus>,
    #[serde(default)]
    head: Option<HeadCommit>,
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
/// render, which is not worth reporting to anyone. Replaced whole, because several Claude
/// Code sessions render at once.
fn store_cache(cache: &[CacheEntry]) {
    let Some(path) = cache_path() else { return };
    let Ok(encoded) = serde_json::to_string(cache) else { return };
    let _ = crate::fs_atomic::write(&path, encoded);
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
    fn the_head_is_read_as_its_time_and_its_distance_from_the_newest_tag() {
        assert_eq!(
            parse_head("1789135495\0v1.1.0-6-gde36776\n"),
            Some(HeadCommit {
                committed_at: 1_789_135_495,
                describe: Some("v1.1.0-6-gde36776".to_string())
            })
        );
        assert_eq!(parse_head("1789135495\0\n").unwrap().describe, None, "no tags at all");
    }

    #[test]
    fn an_operation_left_in_progress_and_the_stash_are_read_from_files() {
        let root = std::env::temp_dir().join(format!(
            "quotastation-git-operation-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond()
        ));
        let rebase = root.join(".git/rebase-merge");
        std::fs::create_dir_all(&rebase).unwrap();
        std::fs::write(rebase.join("msgnum"), "3\n").unwrap();
        std::fs::write(rebase.join("end"), "7\n").unwrap();
        assert_eq!(operation(&root).as_deref(), Some("REBASE 3/7"));

        std::fs::remove_dir_all(&rebase).unwrap();
        std::fs::write(root.join(".git/MERGE_HEAD"), "abc\n").unwrap();
        assert_eq!(operation(&root).as_deref(), Some("MERGE"));

        assert_eq!(stash_count(&root), None, "no stash has ever been made");
        std::fs::create_dir_all(root.join(".git/logs/refs")).unwrap();
        std::fs::write(root.join(".git/logs/refs/stash"), "a b\nc d\n").unwrap();
        assert_eq!(stash_count(&root), Some(2));

        std::fs::remove_dir_all(root).unwrap();
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
