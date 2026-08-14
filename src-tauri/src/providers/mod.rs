pub mod claude;
pub mod codex;

use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

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

pub async fn read_history(kind: ProviderKind) -> Result<(HistorySnapshot, String)> {
    let before = usage_file_state(kind)?;
    let timezone = jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string();
    let history = match kind {
        ProviderKind::Codex => codex::read_history(&timezone).await,
        ProviderKind::Claude => claude::read_history(&timezone).await,
    }?;
    let quality = inspect_history_quality(kind, before.keys())?;
    let after = usage_file_state(kind)?;
    anyhow::ensure!(before == after, "Provider session files changed during parsing; retrying after they settle");
    ensure_history_quality(quality)?;
    anyhow::ensure!(
        before.is_empty() || !history.days.is_empty(),
        "Provider session files exist but no usage records could be parsed"
    );
    Ok((history, timezone))
}

const QUALITY_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct HistoryParseQuality {
    scanned_files: usize,
    candidate_records: usize,
    parsed: usize,
    failed: usize,
}

fn inspect_history_quality<'a>(
    kind: ProviderKind,
    files: impl Iterator<Item = &'a PathBuf>,
) -> Result<HistoryParseQuality> {
    let mut quality = HistoryParseQuality::default();
    for path in files {
        let content = read_quality_tail(path)?;
        quality.scanned_files += 1;
        for line in content.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
            let Some(valid) = candidate_line_is_compatible(kind, line) else { continue };
            quality.candidate_records += 1;
            if valid {
                quality.parsed += 1;
            } else {
                quality.failed += 1;
            }
        }
    }
    Ok(quality)
}

fn read_quality_tail(path: &Path) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(QUALITY_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut content = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut content)?;
    if start > 0 {
        let Some(first_newline) = content.iter().position(|byte| *byte == b'\n') else {
            return Ok(Vec::new());
        };
        content.drain(..=first_newline);
    }
    Ok(content)
}

fn candidate_line_is_compatible(kind: ProviderKind, line: &[u8]) -> Option<bool> {
    let has_marker = match kind {
        ProviderKind::Codex => [b"\"usage\"".as_slice(), b"\"last_token_usage\"", b"\"total_token_usage\""]
            .into_iter()
            .any(|marker| contains_bytes(line, marker)),
        ProviderKind::Claude => contains_bytes(line, b"\"usage\""),
    };
    if !has_marker {
        return None;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
        return Some(false);
    };
    if !contains_token_usage(&value) {
        return None;
    }
    Some(match kind {
        ProviderKind::Codex => codex_usage_shape_is_known(&value),
        ProviderKind::Claude => claude_usage_shape_is_known(&value),
    })
}

fn contains_bytes(content: &[u8], needle: &[u8]) -> bool {
    content.windows(needle.len()).any(|window| window == needle)
}

fn contains_token_usage(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "usage" || key.ends_with("token_usage")) && token_object_is_numeric(value)
                || contains_token_usage(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_token_usage),
        _ => false,
    }
}

fn token_object_is_numeric(value: &serde_json::Value) -> bool {
    let Some(fields) = value.as_object() else { return false };
    [
        "input_tokens",
        "prompt_tokens",
        "output_tokens",
        "completion_tokens",
        "cached_input_tokens",
        "cache_read_input_tokens",
    ]
    .into_iter()
    .any(|key| fields.get(key).is_some_and(serde_json::Value::is_number))
}

fn codex_usage_shape_is_known(value: &serde_json::Value) -> bool {
    [
        "/payload/info/last_token_usage",
        "/payload/info/total_token_usage",
        "/usage",
        "/data/usage",
        "/result/usage",
        "/response/usage",
    ]
    .into_iter()
    .any(|pointer| value.pointer(pointer).is_some_and(token_object_is_numeric))
}

fn claude_usage_shape_is_known(value: &serde_json::Value) -> bool {
    let direct = value.pointer("/message/usage");
    let agent = value.pointer("/data/message/message/usage");
    (direct.or(agent).is_some_and(token_object_is_numeric))
        && direct
            .and_then(|_| value.get("timestamp"))
            .or_else(|| value.pointer("/data/message/timestamp"))
            .is_some_and(serde_json::Value::is_string)
}

fn ensure_history_quality(quality: HistoryParseQuality) -> Result<()> {
    let incompatible = quality.candidate_records >= 3
        && (quality.parsed == 0 || quality.failed * 4 >= quality.candidate_records);
    anyhow::ensure!(
        !incompatible,
        "schema_incompatible: history parser quality degraded \
         (scanned_files={}, candidate_records={}, parsed={}, failed={})",
        quality.scanned_files,
        quality.candidate_records,
        quality.parsed,
        quality.failed
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_check_accepts_known_provider_shapes() {
        let codex = br#"{"timestamp":"2026-08-15T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2}}}}"#;
        let claude = br#"{"timestamp":"2026-08-15T00:00:00Z","message":{"usage":{"input_tokens":10,"output_tokens":2}}}"#;
        assert_eq!(candidate_line_is_compatible(ProviderKind::Codex, codex), Some(true));
        assert_eq!(candidate_line_is_compatible(ProviderKind::Claude, claude), Some(true));
    }

    #[test]
    fn quality_check_rejects_moved_or_malformed_candidate_records() {
        let moved = br#"{"timestamp":"2026-08-15T00:00:00Z","record":{"usage":{"input_tokens":10}}}"#;
        let malformed = br#"{"message":{"usage":{"input_tokens":10}"#;
        assert_eq!(candidate_line_is_compatible(ProviderKind::Claude, moved), Some(false));
        assert_eq!(candidate_line_is_compatible(ProviderKind::Claude, malformed), Some(false));
    }

    #[test]
    fn quality_threshold_tolerates_noise_but_rejects_format_drift() {
        assert!(ensure_history_quality(HistoryParseQuality {
            scanned_files: 1,
            candidate_records: 1,
            parsed: 0,
            failed: 1,
        })
        .is_ok());
        assert!(ensure_history_quality(HistoryParseQuality {
            scanned_files: 2,
            candidate_records: 20,
            parsed: 18,
            failed: 2,
        })
        .is_ok());
        let error = ensure_history_quality(HistoryParseQuality {
            scanned_files: 2,
            candidate_records: 12,
            parsed: 8,
            failed: 4,
        })
        .expect_err("one quarter failed candidates must reject the refresh");
        assert!(error.to_string().contains("scanned_files=2"));
        assert!(error.to_string().contains("failed=4"));
    }
}
