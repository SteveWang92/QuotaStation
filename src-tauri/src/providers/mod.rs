pub mod claude;
pub mod codex;

use std::{path::PathBuf, time::Duration};

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

    /// How often live quota may be read. Codex answers from a local process, so it is
    /// polled as often as the display needs. Claude's usage endpoint rate-limits reads
    /// over a window of tens of minutes and answers a five-minute poll with a refusal,
    /// so it is read far less often; its windows are five hours and seven days, which a
    /// quarter-hour reading still tracks closely.
    pub fn live_refresh_interval(self) -> Duration {
        match self {
            ProviderKind::Codex => Duration::from_secs(300),
            ProviderKind::Claude => Duration::from_secs(900),
        }
    }

    /// The acquisition-path identifiers recorded against refresh runs.
    pub fn live_path(self) -> String {
        format!("{}_live", self.key())
    }

    pub fn history_path(self) -> String {
        format!("{}_history", self.key())
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
    match kind {
        ProviderKind::Codex => codex::read_history().await,
        ProviderKind::Claude => claude::read_history().await,
    }
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
