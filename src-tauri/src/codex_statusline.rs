//! Narrow management of Codex's native status surfaces. Codex has no status-line command
//! hook, so QuotaStation configures only its documented footer and terminal-title arrays.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

const STATUS_LINE: &[&str] = &[
    "model-with-reasoning",
    "git-branch",
    "pull-request-number",
    "branch-changes",
    "run-state",
    "context-used",
    "five-hour-limit",
    "weekly-limit",
];
const TERMINAL_TITLE: &[&str] = &["activity", "project-name"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusLineStatus {
    pub configured: bool,
    pub status_line: Vec<String>,
    pub terminal_title: Vec<String>,
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".codex")))
        .map(|home| home.join("config.toml"))
}

fn read_items(document: &DocumentMut, key: &str) -> Vec<String> {
    document
        .get("tui")
        .and_then(Item::as_table)
        .and_then(|tui| tui.get(key))
        .and_then(Item::as_array)
        .map(|items| {
            items.iter().filter_map(|item| item.as_str().map(ToString::to_string)).collect()
        })
        .unwrap_or_default()
}

pub fn status() -> Result<StatusLineStatus> {
    let Some(path) = config_path() else {
        return Ok(StatusLineStatus {
            configured: false,
            status_line: Vec::new(),
            terminal_title: Vec::new(),
        });
    };
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let document = content.parse::<DocumentMut>().context("read Codex config.toml")?;
    Ok(StatusLineStatus {
        configured: path.exists(),
        status_line: read_items(&document, "status_line"),
        terminal_title: read_items(&document, "terminal_title"),
    })
}

fn string_array(items: &[&str]) -> Item {
    let mut values = Array::new();
    for item in items {
        values.push(*item);
    }
    Item::Value(Value::Array(values))
}

/// Applies the compact native layout: project lives in the terminal title, freeing the
/// single footer row for active work state and account limits.
pub fn apply_layout() -> Result<StatusLineStatus> {
    let path = config_path().context("Codex home is not available")?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut document = content.parse::<DocumentMut>().context("read Codex config.toml")?;
    if !document["tui"].is_table() {
        document["tui"] = Item::Table(Table::new());
    }
    let tui = document["tui"].as_table_mut().context("Codex tui configuration is not a table")?;
    tui.insert("status_line", string_array(STATUS_LINE));
    tui.insert("terminal_title", string_array(TERMINAL_TITLE));
    let staging = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&staging, document.to_string())
        .context("write Codex status-line configuration")?;
    std::fs::rename(&staging, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })?;
    status()
}

#[cfg(test)]
mod tests {
    use super::read_items;
    use toml_edit::DocumentMut;

    #[test]
    fn an_unconfigured_tui_has_no_status_items() {
        let document = DocumentMut::new();
        assert!(read_items(&document, "status_line").is_empty());
        assert!(read_items(&document, "terminal_title").is_empty());
    }
}
