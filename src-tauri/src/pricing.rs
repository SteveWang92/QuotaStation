//! The pricing catalog costs are estimated from: the LiteLLM catalog embedded at build time,
//! or a newer one downloaded when the user asks for it in Settings.
//!
//! Nothing is downloaded unless asked. A download names the LiteLLM commit it came from, and
//! is used only while that commit is newer than the one the build embeds, so updating the
//! application never leaves it pricing from an older download. ccusage prices both providers
//! through its own refresh path, which asks an installed fetcher for the LiteLLM catalog; the
//! fetcher installed here answers from the downloaded file and never from the network.

use std::{path::PathBuf, sync::RwLock};

use anyhow::{Context, Result, bail};
use ccusage_core::PricingMap;
use serde::{Deserialize, Serialize};

use crate::domain::PRICING_CATALOG_REVISION;

/// The version of the downloaded file's own layout. A file stating a later one was written
/// by a newer build and is left unused rather than guessed at.
const FORMAT_VERSION: u32 = 1;
const FILE_NAME: &str = "pricing-catalog.json";
const CATALOG_PATH: &str = "model_prices_and_context_window.json";
const LATEST_COMMIT_URL: &str = "https://api.github.com/repos/BerriAI/litellm/commits?path=model_prices_and_context_window.json&per_page=1";
const EMBEDDED_COMMITTED_AT: &str = env!("QUOTASTATION_PRICING_COMMITTED");

/// The model identifiers ccusage embeds from the catalog; the rest of it prices nothing
/// either provider reports.
const MODEL_PREFIXES: [&str; 13] = [
    "claude-",
    "anthropic.",
    "anthropic/",
    "us.anthropic.",
    "eu.anthropic.",
    "global.anthropic.",
    "jp.anthropic.",
    "au.anthropic.",
    "gpt-",
    "openai/",
    "azure/",
    "zai/",
    "openrouter/openai/",
];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadedFile {
    format_version: u32,
    revision: String,
    /// When the LiteLLM commit was made, in Unix seconds, which is what decides whether it
    /// is newer than the embedded catalog.
    committed_at: i64,
    downloaded_at: String,
    catalog: serde_json::Map<String, serde_json::Value>,
}

struct Downloaded {
    revision: String,
    committed_at: i64,
    downloaded_at: String,
    catalog: String,
}

/// The download in use, when there is one newer than the embedded catalog.
static ACTIVE: RwLock<Option<Downloaded>> = RwLock::new(None);

/// Which catalog prices costs now, as Settings shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingCatalog {
    pub revision: String,
    pub committed_at: i64,
    /// `None` while the embedded catalog is in use.
    pub downloaded_at: Option<String>,
}

fn embedded_committed_at() -> i64 {
    EMBEDDED_COMMITTED_AT.parse().expect("build.rs writes the pin's date as Unix seconds")
}

fn file_path() -> Option<PathBuf> {
    crate::providers::claude::statusline::app_data_dir().map(|dir| dir.join(FILE_NAME))
}

/// Installs the fetcher ccusage prices through, and takes up a download kept from earlier.
pub fn start() {
    ccusage_core::pricing::set_json_fetcher(fetch_catalog);
    let Some(path) = file_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    match parse_file(&text) {
        Ok(Some(downloaded)) => activate(downloaded),
        Ok(None) => {}
        Err(error) => {
            crate::log::write(format!("the downloaded pricing catalog is unusable: {error:#}"))
        }
    }
}

/// The download a file holds, or `None` when it is not newer than the embedded catalog.
fn parse_file(text: &str) -> Result<Option<Downloaded>> {
    let file: DownloadedFile = serde_json::from_str(text).context("not a pricing catalog file")?;
    if file.format_version > FORMAT_VERSION {
        bail!("written by a newer QuotaStation (format {})", file.format_version);
    }
    if file.committed_at <= embedded_committed_at() {
        return Ok(None);
    }
    Ok(Some(Downloaded {
        revision: file.revision,
        committed_at: file.committed_at,
        downloaded_at: file.downloaded_at,
        catalog: serde_json::to_string(&file.catalog)?,
    }))
}

fn activate(downloaded: Downloaded) {
    crate::log::write(format!(
        "pricing from the downloaded catalog {}",
        &downloaded.revision[..downloaded.revision.len().min(12)]
    ));
    *ACTIVE.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(downloaded);
}

/// ccusage asks for the LiteLLM catalog by its upstream address and for models.dev by its
/// own; only the first has a local answer, and only once something was downloaded.
fn fetch_catalog(url: &str) -> std::io::Result<String> {
    let active = ACTIVE.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    match active.as_ref() {
        Some(downloaded) if url.ends_with(CATALOG_PATH) => Ok(downloaded.catalog.clone()),
        _ => Err(std::io::Error::other("no downloaded pricing catalog")),
    }
}

/// The catalog both providers price from.
pub fn pricing_map() -> PricingMap {
    PricingMap::load_with_overrides(false, false, std::iter::empty())
}

pub fn catalog() -> PricingCatalog {
    let active = ACTIVE.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    match active.as_ref() {
        Some(downloaded) => PricingCatalog {
            revision: downloaded.revision.clone(),
            committed_at: downloaded.committed_at,
            downloaded_at: Some(downloaded.downloaded_at.clone()),
        },
        None => PricingCatalog {
            revision: PRICING_CATALOG_REVISION.to_string(),
            committed_at: embedded_committed_at(),
            downloaded_at: None,
        },
    }
}

/// The revision a figure priced now is recorded against.
pub fn revision() -> String {
    catalog().revision
}

#[derive(Deserialize)]
struct CommitEntry {
    sha: String,
    commit: CommitDetail,
}

#[derive(Deserialize)]
struct CommitDetail {
    committer: CommitPerson,
}

#[derive(Deserialize)]
struct CommitPerson {
    date: String,
}

/// Downloads the latest LiteLLM catalog and starts pricing from it when it is newer than
/// the one in use. Returns whether it was.
pub async fn update() -> Result<bool> {
    tokio::task::spawn_blocking(update_blocking).await.context("the pricing download stopped")?
}

fn update_blocking() -> Result<bool> {
    let Some(file) = download(catalog().committed_at)? else { return Ok(false) };
    let compact = serde_json::to_string(&file.catalog)?;
    let path = file_path().context("the application data directory is unknown")?;
    crate::fs_atomic::write(&path, serde_json::to_vec(&file)?)
        .context("the downloaded pricing catalog could not be saved")?;
    activate(Downloaded {
        revision: file.revision,
        committed_at: file.committed_at,
        downloaded_at: file.downloaded_at,
        catalog: compact,
    });
    Ok(true)
}

/// The latest LiteLLM catalog, or `None` when its commit is no newer than `in_use`.
fn download(in_use: i64) -> Result<Option<DownloadedFile>> {
    let agent = ureq::Agent::from(
        ureq::Agent::config_builder()
            .tls_config(
                ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build(),
            )
            .user_agent(concat!("QuotaStation/", env!("CARGO_PKG_VERSION")))
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build(),
    );
    let commits: Vec<CommitEntry> = agent
        .get(LATEST_COMMIT_URL)
        .call()
        .context("GitHub did not answer which LiteLLM commit is latest")?
        .body_mut()
        .read_json()
        .context("GitHub's answer about the latest LiteLLM commit could not be read")?;
    let latest = commits.into_iter().next().context("GitHub named no LiteLLM commit")?;
    let committed_at = latest
        .commit
        .committer
        .date
        .parse::<jiff::Timestamp>()
        .context("the latest LiteLLM commit has no readable date")?
        .as_second();
    if committed_at <= in_use {
        return Ok(None);
    }

    let text = agent
        .get(format!(
            "https://raw.githubusercontent.com/BerriAI/litellm/{}/{CATALOG_PATH}",
            latest.sha
        ))
        .call()
        .context("the LiteLLM catalog could not be downloaded")?
        .body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_string()
        .context("the LiteLLM catalog download was cut short")?;
    let full: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&text).context("the downloaded LiteLLM catalog is not JSON")?;
    let catalog: serde_json::Map<String, serde_json::Value> = full
        .into_iter()
        .filter(|(model, _)| MODEL_PREFIXES.iter().any(|prefix| model.starts_with(prefix)))
        .collect();
    if PricingMap::default().load_json(&serde_json::to_string(&catalog)?) == 0 {
        bail!("the downloaded LiteLLM catalog prices no model QuotaStation reads");
    }
    Ok(Some(DownloadedFile {
        format_version: FORMAT_VERSION,
        revision: latest.sha,
        committed_at,
        downloaded_at: crate::clock::now().to_string(),
        catalog,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(format_version: u32, committed_at: i64) -> String {
        serde_json::json!({
            "formatVersion": format_version,
            "revision": "abc",
            "committedAt": committed_at,
            "downloadedAt": "2026-10-05T00:00:00Z",
            "catalog": {},
        })
        .to_string()
    }

    #[test]
    fn a_download_older_than_the_embedded_catalog_is_not_used() {
        let embedded = embedded_committed_at();
        assert!(parse_file(&file(FORMAT_VERSION, embedded)).expect("readable").is_none());
        assert!(parse_file(&file(FORMAT_VERSION, embedded + 1)).expect("readable").is_some());
    }

    /// Reaches GitHub, so it runs only when asked for.
    #[test]
    #[ignore]
    fn downloads_a_catalog_that_prices_current_models() {
        let file = download(0).expect("download").expect("a newer catalog");
        let mut map = PricingMap::default();
        map.load_json(&serde_json::to_string(&file.catalog).expect("serialize"));
        assert!(map.find("claude-opus-5-5").is_some());
        assert!(map.find("gpt-5.5").is_some());
    }

    #[test]
    fn a_file_from_a_newer_build_is_left_unused() {
        assert!(parse_file(&file(FORMAT_VERSION + 1, embedded_committed_at() + 1)).is_err());
    }
}
