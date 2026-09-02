use std::{
    collections::BTreeSet,
    env,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    time::{Duration, timeout},
};

use crate::{
    domain::{Freshness, LimitKind, LimitWindow, LiveSnapshot, QuotaLevel, WindowSource},
    providers::{ProviderKind, SignInRequired, is_sign_in_required},
};

const CODEX_EXECUTABLE_OVERRIDE: &str = "QUOTASTATION_CODEX_EXECUTABLE";

/// The app-server is an external child process: it can start and then never answer, and a
/// refresh that waits for it forever holds the live-refresh lock for the rest of the session.
/// The outer bound covers the whole candidate walk, the inner one each candidate's exchange,
/// so a stalled executable is abandoned and the next one still gets its turn.
pub async fn read_live() -> Result<LiveSnapshot> {
    timeout(Duration::from_secs(12), read_live_inner())
        .await
        .context("Codex app-server timed out")?
}

async fn read_live_inner() -> Result<LiveSnapshot> {
    let candidates = discover_codex_candidates()?;
    // How many were found, never which: the answer is a list of executable paths, and a
    // path is what this log exists not to carry.
    crate::log::write(format!("codex app-server: {} candidate(s) found", candidates.len()));
    first_success(candidates, attempt_candidate).await
}

async fn attempt_candidate(executable: PathBuf) -> Result<LiveSnapshot> {
    let mut child = spawn_app_server(&executable)?;
    let result = timeout(Duration::from_secs(4), exchange(&mut child)).await;
    let _ = child.kill().await;
    let result = result.context("Codex app-server candidate timed out")?;
    if let Err(error) = &result {
        crate::log::write(format!("codex app-server candidate declined: {error:#}"));
    }
    result
}

async fn first_success<T, F, Fut>(candidates: Vec<PathBuf>, mut attempt: F) -> Result<T>
where
    F: FnMut(PathBuf) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last_error = None;
    for executable in candidates {
        match attempt(executable).await {
            Ok(value) => return Ok(value),
            // An expired sign-in is the account's answer, not this executable's: the next
            // candidate reads the same credentials and would only bury that answer under
            // whatever it fails with.
            Err(error) if is_sign_in_required(&error) => return Err(error),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap())
}

fn spawn_app_server(executable: &Path) -> Result<Child> {
    let mut command = Command::new(executable);
    command
        .args(["app-server", "--stdio"])
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
        if path.is_file() {
            return Ok(vec![path]);
        }
        bail!("Codex executable override does not point to a file");
    }
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    if let Ok(paths) = which::which_all("codex") {
        for path in paths {
            add_if_file(&mut candidates, &mut seen, path);
        }
    }
    if let Some(app_data) = env::var_os("APPDATA").map(PathBuf::from) {
        add_if_file(&mut candidates, &mut seen, app_data.join("npm").join("codex.cmd"));
        let nvm = app_data.join("nvm");
        if let Ok(versions) = std::fs::read_dir(nvm) {
            let mut versions =
                versions.filter_map(Result::ok).map(|entry| entry.path()).collect::<Vec<_>>();
            versions.sort_by_key(|path| std::cmp::Reverse(version_key(path)));
            for version in versions {
                add_if_file(&mut candidates, &mut seen, version.join("codex.cmd"));
                add_if_file(&mut candidates, &mut seen, version.join("codex.exe"));
            }
        }
    }
    if candidates.is_empty() {
        bail!(
            "Codex CLI was not found; install the official @openai/codex package or set QUOTASTATION_CODEX_EXECUTABLE"
        );
    }
    Ok(candidates)
}

fn version_key(path: &Path) -> Vec<u64> {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn add_if_file(candidates: &mut Vec<PathBuf>, seen: &mut BTreeSet<PathBuf>, path: PathBuf) {
    if path.is_file() && seen.insert(path.clone()) {
        candidates.push(path);
    }
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
    send(
        &mut stdin,
        json!({ "method": "account/read", "id": 2, "params": { "refreshToken": false } }),
    )
    .await?;
    send(&mut stdin, json!({ "method": "account/rateLimits/read", "id": 3 })).await?;

    let mut account = None;
    let mut rate_limits = None;
    while account.is_none() || rate_limits.is_none() {
        let value = next_value(&mut lines).await?;
        match value.get("id").and_then(Value::as_i64) {
            Some(2) => account = Some(response_result(value)?),
            Some(3) => rate_limits = Some(response_result(value)?),
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

async fn next_value(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> Result<Value> {
    let line = lines
        .next_line()
        .await?
        .ok_or_else(|| anyhow!("Codex app-server closed the connection"))?;
    serde_json::from_str(&line).context("decode Codex app-server response")
}

fn response_result(value: Value) -> Result<Value> {
    if let Some(error) = value.get("error") {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("unknown error");
        if reports_expired_sign_in(message) {
            return Err(anyhow::Error::new(SignInRequired(ProviderKind::Codex)));
        }
        bail!("Codex app-server request failed: {message}");
    }
    value.get("result").cloned().ok_or_else(|| anyhow!("Codex app-server response omitted result"))
}

/// Whether the app-server refused because the stored credentials are no longer good.
///
/// It reports this by relaying the backend's own 401, so the reply is a transport error
/// carrying the upstream body rather than a code of its own: `token_expired` is what that
/// body names the condition, and the status covers the wording changing around it.
fn reports_expired_sign_in(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("token_expired") || message.contains("401 unauthorized")
}

fn normalize(account: Value, rate_result: Value) -> Result<LiveSnapshot> {
    let plan_type =
        account.pointer("/account/planType").and_then(Value::as_str).map(str::to_string);
    let rate_limits = rate_result
        .get("rateLimits")
        .and_then(Value::as_object)
        .context("schema_incompatible: rate-limit response omitted rateLimits")?;
    let mut limits = Vec::new();
    let observed_at = jiff::Timestamp::now().as_second();
    for (field, kind) in [("primary", LimitKind::Primary), ("secondary", LimitKind::Secondary)] {
        let Some(value) = rate_limits.get(field) else { continue };
        if value.is_null() {
            continue;
        }
        let value = value
            .as_object()
            .with_context(|| format!("schema_incompatible: {field} bucket is not an object"))?;
        let used_percent = value
            .get("usedPercent")
            .and_then(Value::as_f64)
            .with_context(|| format!("schema_incompatible: {field} bucket omitted usedPercent"))?;
        let minutes = value
            .get("windowDurationMins")
            .and_then(Value::as_i64)
            .filter(|minutes| *minutes > 0)
            .with_context(|| {
                format!("schema_incompatible: {field} bucket omitted a valid window duration")
            })?;
        let resets_at = value
            .get("resetsAt")
            .and_then(Value::as_i64)
            .with_context(|| format!("schema_incompatible: {field} bucket omitted resetsAt"))?;
        if !used_percent.is_finite() || !(0.0..=100.0).contains(&used_percent) {
            bail!("schema_incompatible: {field} bucket has an invalid percentage");
        }
        if resets_at < observed_at - 60 || resets_at > observed_at + minutes * 60 * 2 {
            bail!("schema_incompatible: {field} bucket has an invalid reset time");
        }
        limits.push(LimitWindow {
            kind,
            label: kind.window_label(Some(minutes)),
            used_percent: Some(used_percent),
            window_duration_mins: Some(minutes),
            resets_at: Some(resets_at),
            source: WindowSource::AppServer,
            observed_at,
            freshness: Freshness::Fresh,
            status_level: QuotaLevel::Healthy,
        });
    }
    if limits.is_empty() {
        bail!("schema_incompatible: rate-limit response contained no known buckets");
    }
    Ok(LiveSnapshot {
        plan_type,
        limits,
        earned_reset_count: rate_result
            .pointer("/rateLimitResetCredits/availableCount")
            .and_then(Value::as_u64),
        earned_reset_expires_at: earliest_credit_expiry(&rate_result),
        // Codex publishes its own percentages, so nothing needs corroborating.
    })
}

/// When the first of the available reset credits stops being redeemable.
///
/// The detail rows are optional and the backend may cap them, so the count beside this is
/// the authoritative total and this is only the soonest deadline among the rows it sent.
/// A credit that never expires carries no `expiresAt` and contributes no deadline.
fn earliest_credit_expiry(rate_result: &Value) -> Option<i64> {
    rate_result
        .pointer("/rateLimitResetCredits/credits")?
        .as_array()?
        .iter()
        .filter(|credit| credit.get("status").and_then(Value::as_str) == Some("available"))
        .filter_map(|credit| credit.get("expiresAt").and_then(Value::as_i64))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_exchange_falls_back_to_the_next_candidate() {
        let candidates = vec![PathBuf::from("first"), PathBuf::from("second")];
        let snapshot = first_success(candidates, |candidate| async move {
            if candidate == Path::new("first") {
                bail!("protocol handshake failed");
            }
            Ok(LiveSnapshot {
                plan_type: Some("test".to_string()),
                limits: Vec::new(),
                earned_reset_count: None,
                earned_reset_expires_at: None,
            })
        })
        .await
        .expect("second candidate succeeds");
        assert_eq!(snapshot.plan_type.as_deref(), Some("test"));
    }

    #[test]
    fn an_expired_sign_in_is_told_apart_from_a_broken_read() {
        let expired = json!({
            "error": {
                "code": -32603,
                "message": "failed to fetch codex rate limits: GET https://chatgpt.com/backend-api/wham/usage failed: 401 Unauthorized; content-type=text/plain; body={\"error\":{\"code\":\"token_expired\"}}",
            },
            "id": 3,
        });
        let error = response_result(expired).expect_err("an expired sign-in is refused");
        assert!(is_sign_in_required(&error), "it is reported as a sign-in rather than a fault");
        let broken = json!({ "error": { "message": "internal error" }, "id": 3 });
        let error = response_result(broken).expect_err("a broken read is still refused");
        assert!(!is_sign_in_required(&error), "an ordinary failure stays an ordinary failure");
    }

    #[test]
    fn nvm_versions_sort_semantically() {
        let mut versions =
            [PathBuf::from("v9.0.0"), PathBuf::from("v24.2.0"), PathBuf::from("v24.18.1")];
        versions.sort_by_key(|path| std::cmp::Reverse(version_key(path)));
        assert_eq!(versions[0], Path::new("v24.18.1"));
    }

    #[test]
    fn the_soonest_available_credit_is_the_one_that_expires() {
        let credits = json!({
            "rateLimitResetCredits": {
                "availableCount": 2,
                "credits": [
                    { "status": "available", "expiresAt": 1_784_246_400_i64 },
                    { "status": "available", "expiresAt": 1_781_654_400_i64 },
                    { "status": "redeemed", "expiresAt": 1_000_000_i64 },
                    { "status": "available", "expiresAt": null },
                ],
            },
        });
        assert_eq!(earliest_credit_expiry(&credits), Some(1_781_654_400));
    }

    #[test]
    fn a_credit_count_with_no_detail_rows_reports_no_expiry() {
        let summary = json!({ "rateLimitResetCredits": { "availableCount": 2 } });
        assert_eq!(earliest_credit_expiry(&summary), None);
    }

    #[test]
    fn missing_known_rate_limit_buckets_are_schema_incompatible() {
        let error = normalize(json!({ "account": {} }), json!({ "rateLimits": {} }))
            .expect_err("empty rate-limit shape must fail");
        assert!(error.to_string().contains("schema_incompatible"));
    }
}
