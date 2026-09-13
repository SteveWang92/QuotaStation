//! Replacing a file whole.
//!
//! Several processes read the files QuotaStation writes while it writes them: the status-line
//! bridge, Claude Code, a sync client. Each write is staged beside its target under this
//! process's own name and renamed onto it, so a reader sees the old content or the new one and
//! never half of either.

use std::path::Path;

/// Writes `contents` to `path` by staging and renaming, creating the parent directory first.
/// A failed rename leaves the previous file untouched and removes the staging copy.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut staging = path.as_os_str().to_owned();
    staging.push(format!(".{}.tmp", std::process::id()));
    std::fs::write(&staging, contents)?;
    std::fs::rename(&staging, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })
}
