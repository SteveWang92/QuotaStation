pub mod codex;

use std::path::PathBuf;

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
}

impl ProviderKind {
    /// Declaration order is display order on every surface.
    pub const ALL: [ProviderKind; 1] = [ProviderKind::Codex];

    pub fn key(self) -> &'static str {
        match self {
            ProviderKind::Codex => "codex",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Codex => "Codex",
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
        }
    }
}

/// Two providers is not enough to justify an async trait and the dependency it needs, so
/// acquisition dispatches on the kind instead.
pub async fn read_live(kind: ProviderKind) -> Result<LiveSnapshot> {
    match kind {
        ProviderKind::Codex => codex::read_live().await,
    }
}

pub async fn read_history(kind: ProviderKind) -> Result<HistorySnapshot> {
    match kind {
        ProviderKind::Codex => codex::read_history().await,
    }
}

/// Historical rate-limit readings recovered from a provider's own logs. Only providers
/// that write their server's rate-limit answers locally can offer this.
pub async fn read_observations(
    kind: ProviderKind,
    since: Option<i64>,
) -> Result<Vec<WindowObservation>> {
    match kind {
        ProviderKind::Codex => codex::read_observations(since).await,
    }
}
