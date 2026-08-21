use std::path::{Path, PathBuf};
use std::process::Command;

fn git_dir(repo: &Path) -> Option<PathBuf> {
    let marker = repo.join(".git");
    if marker.is_dir() {
        return Some(marker);
    }
    let pointer = std::fs::read_to_string(marker).ok()?;
    let path = pointer.strip_prefix("gitdir: ")?.trim();
    let path = PathBuf::from(path);
    Some(if path.is_absolute() { path } else { repo.join(path) })
}

fn build_commit(repo: &Path) -> String {
    Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|revision| revision.trim().to_string())
        .filter(|revision| !revision.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn main() {
    let repo = Path::new("..");
    if let Some(git_dir) = git_dir(repo) {
        let head = git_dir.join("HEAD");
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(contents) = std::fs::read_to_string(&head)
            && let Some(reference) = contents.strip_prefix("ref: ").map(str::trim)
        {
            println!("cargo:rerun-if-changed={}", git_dir.join(reference).display());
        }
    }
    println!("cargo:rustc-env=QUOTASTATION_BUILD_COMMIT={}", build_commit(repo));
    let lock_path = "../vendor/ccusage/flake.lock";
    println!("cargo:rerun-if-changed={lock_path}");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    let lock: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(lock_path).expect("read vendored ccusage flake.lock"),
    )
    .expect("parse vendored ccusage flake.lock");
    let revision = lock
        .pointer("/nodes/litellm/locked/rev")
        .and_then(serde_json::Value::as_str)
        .expect("ccusage flake.lock must pin LiteLLM");
    println!("cargo:rustc-env=QUOTASTATION_PRICING_REVISION={revision}");
    tauri_build::build()
}
