pub mod claude;
pub mod codex;

use std::{collections::BTreeMap, path::{Path, PathBuf}, time::{Duration, SystemTime}};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{HistorySnapshot, LiveSnapshot},
    resets::WindowObservation,
};

/// Every provider QuotaStation can monitor. The key doubles as the renderer's identifier
/// and as the `provider_instances.provider` value, so the three layers never disagree
/// about what a provider is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Codex,
    Claude,
}

impl ProviderKind {
    /// Declaration order is display order on every surface.
    pub const ALL: [ProviderKind; 2] = [ProviderKind::Codex, ProviderKind::Claude];

    pub fn key(self) -> &'static str {
        match self {
            ProviderKind::Codex => "codex",
            ProviderKind::Claude => "claude",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Codex => "Codex",
            ProviderKind::Claude => "Claude Code",
        }
    }

    /// How often live quota may be read. Codex answers from a local process; Claude is
    /// read from its session logs, which means parsing them, and a five-hour window that
    /// only moves when a new one opens does not repay doing that every five minutes.
    pub fn live_refresh_interval(self) -> Duration {
        match self {
            ProviderKind::Codex => Duration::from_secs(300),
            ProviderKind::Claude => Duration::from_secs(600),
        }
    }

    /// The acquisition-path identifiers recorded against refresh runs.
    pub fn live_path(self) -> String {
        format!("{}_live", self.key())
    }

    pub fn history_path(self) -> String {
        format!("{}_history", self.key())
    }

    /// Whether this provider's live quota comes from the same session files the history
    /// is parsed from, so a change to them is worth re-reading the quota as well.
    pub fn live_follows_logs(self) -> bool {
        matches!(self, ProviderKind::Claude)
    }

    /// Whether this provider's client has left usage records on this machine. A provider
    /// nobody uses is not shown at all, so the display never carries a column that can
    /// only ever say "unavailable".
    pub fn is_installed(self) -> bool {
        match self {
            ProviderKind::Codex => ccusage_adapter_codex::has_codex_usage_records(),
            ProviderKind::Claude => ccusage_adapter_claude::has_claude_usage_records(),
        }
        .unwrap_or(false)
    }

    /// Directories holding this provider's usage records, for filesystem watching.
    pub fn usage_paths(self) -> Result<Vec<PathBuf>> {
        match self {
            ProviderKind::Codex => ccusage_adapter_codex::codex_usage_paths()
                .map_err(|error| anyhow::anyhow!(error.to_string())),
            ProviderKind::Claude => ccusage_adapter_claude::claude_usage_paths()
                .map_err(|error| anyhow::anyhow!(error.to_string())),
        }
    }
}

/// Two providers is not enough to justify an async trait and the dependency it needs, so
/// acquisition dispatches on the kind instead.
pub async fn read_live(kind: ProviderKind) -> Result<LiveSnapshot> {
    match kind {
        ProviderKind::Codex => codex::read_live().await,
        ProviderKind::Claude => claude::read_live().await,
    }
}

pub async fn read_history(kind: ProviderKind) -> Result<HistorySnapshot> {
    let before = usage_file_state(kind)?;
    let history = match kind {
        ProviderKind::Codex => codex::read_history().await,
        ProviderKind::Claude => claude::read_history().await,
    }?;
    let after = usage_file_state(kind)?;
    anyhow::ensure!(before == after, "Provider session files changed during parsing; retrying after they settle");
    anyhow::ensure!(
        before.is_empty() || !history.days.is_empty(),
        "Provider session files exist but no usage records could be parsed"
    );
    Ok(history)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileState {
    len: u64,
    modified: Option<SystemTime>,
}

fn usage_file_state(kind: ProviderKind) -> Result<BTreeMap<PathBuf, FileState>> {
    let mut state = BTreeMap::new();
    for root in kind.usage_paths()? {
        collect_file_state(&root, &mut state)?;
    }
    Ok(state)
}

fn collect_file_state(path: &Path, state: &mut BTreeMap<PathBuf, FileState>) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_file_state(&path, state)?;
        } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl")) {
            let metadata = entry.metadata()?;
            state.insert(path, FileState { len: metadata.len(), modified: metadata.modified().ok() });
        }
    }
    Ok(())
}

/// Historical rate-limit readings recovered from a provider's own logs. Only providers
/// that write their server's rate-limit answers locally can offer this. Claude Code
/// records a reset time only in the error it raises once a limit is already reached,
/// with no usage percentage, which is not enough to recognise a restart.
pub async fn read_observations(
    kind: ProviderKind,
    since: Option<i64>,
) -> Result<Vec<WindowObservation>> {
    match kind {
        ProviderKind::Codex => codex::read_observations(since).await,
        ProviderKind::Claude => Ok(Vec::new()),
    }
}
