use std::{collections::BTreeSet, env, path::{Path, PathBuf}, process::Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    time::{Duration, timeout},
};

use crate::domain::{LimitKind, LimitWindow, LiveSnapshot};

const CODEX_EXECUTABLE_OVERRIDE: &str = "QUOTASTATION_CODEX_EXECUTABLE";

pub async fn read_live() -> Result<LiveSnapshot> {
    timeout(Duration::from_secs(12), read_live_inner())
        .await
        .context("Codex app-server timed out")?
}

async fn read_live_inner() -> Result<LiveSnapshot> {
    let candidates = discover_codex_candidates()?;
    let mut spawn_error = None;
    for executable in candidates {
        match spawn_app_server(&executable) {
            Ok(mut child) => {
                let result = exchange(&mut child).await;
                let _ = child.kill().await;
                return result;
            }
            Err(error) => spawn_error = Some(error),
        }
    }
    Err(spawn_error.unwrap_or_else(|| anyhow!("No usable Codex app-server executable was found")))
}

fn spawn_app_server(executable: &Path) -> Result<Child> {
    let mut command = Command::new(executable);
    command.args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.spawn().context("start installed Codex app-server")
}

fn discover_codex_candidates() -> Result<Vec<PathBuf>> {
    if let Some(path) = env::var_os(CODEX_EXECUTABLE_OVERRIDE).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() { return Ok(vec![path]); }
        bail!("Codex executable override does not point to a file");
    }
    let mut candidates = BTreeSet::new();
    if let Some(app_data) = env::var_os("APPDATA").map(PathBuf::from) {
        add_if_file(&mut candidates, app_data.join("npm").join("codex.cmd"));
        let nvm = app_data.join("nvm");
        if let Ok(versions) = std::fs::read_dir(nvm) {
            let mut versions = versions.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>();
            versions.sort_by(|left, right| right.cmp(left));
            for version in versions {
                add_if_file(&mut candidates, version.join("codex.cmd"));
                add_if_file(&mut candidates, version.join("codex.exe"));
            }
        }
    }
    if let Ok(paths) = which::which_all("codex") {
        candidates.extend(paths);
    }
    if candidates.is_empty() {
        bail!("Codex CLI was not found; install the official @openai/codex package or set QUOTASTATION_CODEX_EXECUTABLE");
    }
    Ok(candidates.into_iter().collect())
}

fn add_if_file(candidates: &mut BTreeSet<PathBuf>, path: PathBuf) {
    if path.is_file() { candidates.insert(path); }
}

async fn exchange(child: &mut Child) -> Result<LiveSnapshot> {
    let mut stdin = child.stdin.take().context("open Codex app-server stdin")?;
    let stdout = child.stdout.take().context("open Codex app-server stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    send(&mut stdin, json!({
        "method": "initialize",
        "id": 1,
        "params": { "clientInfo": { "name": "quotastation", "title": "QuotaStation", "version": env!("CARGO_PKG_VERSION") } }
    })).await?;
    wait_for_id(&mut lines, 1).await?;
    send(&mut stdin, json!({ "method": "initialized", "params": {} })).await?;
    send(&mut stdin, json!({ "method": "account/read", "id": 2, "params": { "refreshToken": false } })).await?;
    send(&mut stdin, json!({ "method": "account/rateLimits/read", "id": 3 })).await?;
    send(&mut stdin, json!({ "method": "account/usage/read", "id": 4 })).await?;

    let mut account = None;
    let mut rate_limits = None;
    let mut usage_seen = false;
    while account.is_none() || rate_limits.is_none() || !usage_seen {
        let value = next_value(&mut lines).await?;
        match value.get("id").and_then(Value::as_i64) {
            Some(2) => account = Some(response_result(value)?),
            Some(3) => rate_limits = Some(response_result(value)?),
            Some(4) => { usage_seen = true; }
            _ => {}
        }
    }
    normalize(account.unwrap(), rate_limits.unwrap())
}

async fn send(stdin: &mut tokio::process::ChildStdin, value: Value) -> Result<()> {
    stdin.write_all(serde_json::to_string(&value)?.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

async fn wait_for_id(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: i64,
) -> Result<Value> {
    loop {
        let value = next_value(lines).await?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return response_result(value);
        }
    }
}

async fn next_value(lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>) -> Result<Value> {
    let line = lines.next_line().await?.ok_or_else(|| anyhow!("Codex app-server closed the connection"))?;
    serde_json::from_str(&line).context("decode Codex app-server response")
}

fn response_result(value: Value) -> Result<Value> {
    if let Some(error) = value.get("error") {
        bail!("Codex app-server request failed: {}", error.get("message").and_then(Value::as_str).unwrap_or("unknown error"));
    }
    value.get("result").cloned().ok_or_else(|| anyhow!("Codex app-server response omitted result"))
}

fn normalize(account: Value, rate_result: Value) -> Result<LiveSnapshot> {
    let plan_type = account.pointer("/account/planType").and_then(Value::as_str).map(str::to_string);
    let rate_limits = rate_result.get("rateLimits").unwrap_or(&Value::Null);
    let mut limits = Vec::new();
    for (field, kind) in [("primary", LimitKind::Primary), ("secondary", LimitKind::Secondary)] {
        let value = rate_limits.get(field).unwrap_or(&Value::Null);
        if !value.is_object() { continue; }
        let minutes = value.get("windowDurationMins").and_then(Value::as_i64);
        limits.push(LimitWindow {
            kind,
            label: kind.window_label(minutes),
            used_percent: value.get("usedPercent").and_then(Value::as_f64),
            remaining_percent: value.get("usedPercent").and_then(Value::as_f64).map(|used| (100.0 - used).clamp(0.0, 100.0)),
            window_duration_mins: minutes,
            resets_at: value.get("resetsAt").and_then(Value::as_i64),
        });
    }
    Ok(LiveSnapshot {
        plan_type,
        limits,
        earned_reset_count: rate_result.pointer("/rateLimitResetCredits/availableCount").and_then(Value::as_u64),
        // Codex publishes its own percentages, so nothing needs corroborating.
    })
}
