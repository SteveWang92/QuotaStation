//! Live Claude Code quota.
//!
//! Claude Code ships no local read interface comparable to `codex app-server`, and
//! nothing under `~/.claude` records remaining quota or a forward-looking reset. The one
//! authoritative source is Anthropic's own OAuth usage endpoint, so this adapter reads
//! the access token Claude Code already stored and presents it to that endpoint.
//!
//! The token is read here and nowhere else. It is never persisted, never logged, never
//! included in diagnostics, and never crosses into the renderer. This adapter also never
//! refreshes the token itself and never sends a chat request to infer limits from
//! response headers, because both would mutate account state that QuotaStation only reads.

use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use tokio::time::{Duration, timeout};

use crate::domain::{LimitKind, LimitWindow};

use super::{FIVE_HOUR_WINDOW_MINS, SEVEN_DAY_WINDOW_MINS, claude_home};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

const SIGN_IN_REQUIRED: &str =
    "Claude Code is not signed in. Run `claude auth login`, then refresh.";
const SIGN_IN_EXPIRED: &str =
    "The Claude Code sign-in has expired. Run `claude auth login`, then refresh.";

#[derive(Deserialize)]
struct UsageResponse {
    five_hour: Option<UsageBucket>,
    seven_day: Option<UsageBucket>,
}

#[derive(Deserialize)]
struct UsageBucket {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

/// When the endpoint last asked to be left alone, as an epoch second. The usage endpoint
/// rate-limits reads over a window longer than the five-minute refresh interval, so
/// honouring its `Retry-After` is what stops a scheduled poll from hammering it and
/// keeping the account permanently limited.
static RETRY_AFTER_UNTIL: AtomicI64 = AtomicI64::new(0);

/// The shortest gap between two reads of the endpoint, as an epoch second and a duration.
/// Claude Code polls the same endpoint on its own schedule; reading it more often than
/// this wins nothing and costs both readers the budget they share.
const MIN_ATTEMPT_INTERVAL_SECS: i64 = 900;
static LAST_ATTEMPT_AT: AtomicI64 = AtomicI64::new(0);

/// The plan recorded next to the sign-in token. Reading it involves no request and no
/// credential leaving this machine, so the session-log path can name the plan too.
pub fn plan_type() -> Option<String> {
    read_credentials().ok().and_then(|credentials| credentials.plan_type)
}

/// The quota windows as Anthropic itself reports them, or why they could not be read.
///
/// Claude Code reads this same endpoint for its own usage display, and the endpoint
/// rate-limits an account as a whole, so two readers compete for one small budget. This
/// one therefore yields: it never asks more often than [`MIN_ATTEMPT_INTERVAL_SECS`],
/// always waits out a `Retry-After`, and its failures leave the session-log windows in
/// place rather than becoming the provider's state.
pub async fn read_windows() -> Result<Vec<LimitWindow>> {
    timeout(REQUEST_TIMEOUT, read_windows_inner())
        .await
        .context("Claude usage read timed out")?
}

async fn read_windows_inner() -> Result<Vec<LimitWindow>> {
    let now = jiff::Timestamp::now().as_second();
    let retry_at = RETRY_AFTER_UNTIL.load(Ordering::Relaxed);
    if retry_at > now {
        bail!("{}", rate_limited_message(Some(retry_at - now)));
    }
    let last_attempt = LAST_ATTEMPT_AT.load(Ordering::Relaxed);
    if last_attempt > 0 && now - last_attempt < MIN_ATTEMPT_INTERVAL_SECS {
        bail!(
            "Waiting before the next usage read so Claude Code's own reads are not crowded out."
        );
    }
    let credentials = read_credentials()?;
    if credentials.is_expired() {
        bail!("{SIGN_IN_EXPIRED}");
    }
    LAST_ATTEMPT_AT.store(now, Ordering::Relaxed);
    let response = fetch_usage(&credentials.access_token).await?;
    Ok(normalize(&response))
}

fn normalize(response: &UsageResponse) -> Vec<LimitWindow> {
    [
        (
            response.five_hour.as_ref(),
            LimitKind::Primary,
            FIVE_HOUR_WINDOW_MINS,
        ),
        (
            response.seven_day.as_ref(),
            LimitKind::Secondary,
            SEVEN_DAY_WINDOW_MINS,
        ),
    ]
    .into_iter()
    .filter_map(|(bucket, kind, minutes)| {
        let bucket = bucket?;
        let used_percent = bucket.utilization.map(normalize_utilization);
        Some(LimitWindow {
            kind,
            label: kind.window_label(Some(minutes)),
            used_percent,
            remaining_percent: used_percent.map(|used| (100.0 - used).clamp(0.0, 100.0)),
            window_duration_mins: Some(minutes),
            resets_at: bucket.resets_at.as_deref().and_then(parse_timestamp),
        })
    })
    .collect()
}

/// The OAuth usage endpoint reports utilization on a 0-100 percentage scale. Fractional
/// rate-limit headers use a different protocol and must be normalized at their own boundary.
fn normalize_utilization(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

fn parse_timestamp(value: &str) -> Option<i64> {
    value.parse::<jiff::Timestamp>().ok().map(|ts| ts.as_second())
}

async fn fetch_usage(access_token: &str) -> Result<UsageResponse> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("build Claude usage client")?;
    let response = client
        .get(USAGE_URL)
        .bearer_auth(access_token)
        .header("anthropic-beta", OAUTH_BETA)
        .send()
        .await
        .context("request Claude usage")?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        bail!("{SIGN_IN_EXPIRED}");
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // The usage endpoint rate-limits reads independently of the quota it reports, and
        // asks for a wait measured in minutes. This is not a quota problem and must not be
        // shown as one; the next scheduled refresh picks the reading back up.
        let retry_after = retry_after_seconds(&response);
        if let Some(seconds) = retry_after {
            RETRY_AFTER_UNTIL.store(jiff::Timestamp::now().as_second() + seconds, Ordering::Relaxed);
        }
        bail!("{}", rate_limited_message(retry_after));
    }
    // Any answer at all means the cooldown has passed.
    RETRY_AFTER_UNTIL.store(0, Ordering::Relaxed);
    if !status.is_success() {
        // The body can echo account details, so only the status is reported.
        bail!("Claude usage endpoint returned status {}", status.as_u16());
    }
    response
        .json::<UsageResponse>()
        .await
        .context("decode Claude usage response")
}

fn retry_after_seconds(response: &reqwest::Response) -> Option<i64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|seconds| *seconds > 0)
}

fn rate_limited_message(retry_after: Option<i64>) -> String {
    match retry_after {
        Some(seconds) => format!(
            "Anthropic is rate limiting usage reads; retrying in about {} minutes.",
            (seconds + 59) / 60
        ),
        None => "Anthropic is rate limiting usage reads; the next refresh will retry.".to_string(),
    }
}

struct Credentials {
    access_token: String,
    expires_at_ms: Option<i64>,
    plan_type: Option<String>,
}

impl Credentials {
    /// Only a positive expiry means anything. Claude Code leaves this field at zero on a
    /// live sign-in, so treating any non-positive value as "expired" would reject a
    /// perfectly good token. The endpoint's own 401 remains the authority either way;
    /// this check just avoids a request that is certain to fail.
    fn is_expired(&self) -> bool {
        self.expires_at_ms.is_some_and(|expires_at| {
            expires_at > 0 && expires_at <= jiff::Timestamp::now().as_millisecond()
        })
    }
}

fn read_credentials() -> Result<Credentials> {
    let path = claude_home()
        .map(|home| home.join(".credentials.json"))
        .ok_or_else(|| anyhow!("{SIGN_IN_REQUIRED}"))?;
    if !path.is_file() {
        bail!("{SIGN_IN_REQUIRED}");
    }
    // Deliberately not wrapped with the path: an error string carrying the credentials
    // location would end up in the database and the diagnostics panel.
    let content = std::fs::read_to_string(&path).map_err(|_| anyhow!("{SIGN_IN_REQUIRED}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|_| anyhow!("{SIGN_IN_REQUIRED}"))?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or_else(|| anyhow!("{SIGN_IN_REQUIRED}"))?;
    let access_token = oauth
        .get("accessToken")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("{SIGN_IN_REQUIRED}"))?
        .to_string();
    // The plan is recorded alongside the token, so the tier never has to be recovered
    // from `~/.claude.json`, which also holds the account email and identifiers that
    // QuotaStation must not collect.
    let plan_type = ["subscriptionType", "rateLimitTier"]
        .into_iter()
        .find_map(|field| {
            oauth
                .get(field)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
    Ok(Credentials {
        access_token,
        expires_at_ms: oauth.get("expiresAt").and_then(serde_json::Value::as_i64),
        plan_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(five_hour: f64, seven_day: f64) -> UsageResponse {
        UsageResponse {
            five_hour: Some(UsageBucket {
                utilization: Some(five_hour),
                resets_at: Some("2026-08-12T18:00:00Z".to_string()),
            }),
            seven_day: Some(UsageBucket {
                utilization: Some(seven_day),
                resets_at: None,
            }),
        }
    }

    #[test]
    fn the_two_windows_map_onto_the_shared_quota_vocabulary() {
        let limits = normalize(&response(40.0, 12.0));
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].kind, LimitKind::Primary);
        assert_eq!(limits[0].label, "5-hour window");
        assert_eq!(limits[0].window_duration_mins, Some(FIVE_HOUR_WINDOW_MINS));
        assert_eq!(limits[0].remaining_percent, Some(60.0));
        assert_eq!(limits[1].kind, LimitKind::Secondary);
        assert_eq!(limits[1].label, "Weekly window");
        assert_eq!(limits[1].remaining_percent, Some(88.0));
    }

    #[test]
    fn a_reset_timestamp_is_kept_as_the_server_reported_it() {
        let limits = normalize(&response(40.0, 12.0));
        assert_eq!(limits[0].resets_at, Some(1_786_557_600));
        assert_eq!(limits[1].resets_at, None, "an absent reset stays unknown");
    }

    #[test]
    fn utilization_is_read_as_an_endpoint_percentage() {
        assert_eq!(normalize_utilization(40.0), 40.0);
        assert_eq!(normalize_utilization(0.4), 0.4);
        assert_eq!(normalize_utilization(1.0), 1.0);
        assert_eq!(normalize_utilization(100.0), 100.0);
        assert_eq!(normalize_utilization(140.0), 100.0);
    }

    #[test]
    fn a_rate_limited_read_is_not_reported_as_a_quota_problem() {
        let message = rate_limited_message(Some(628));
        assert!(message.contains("11 minutes"), "{message}");
        assert!(!message.contains("quota"), "a read limit is not a quota limit");
        assert!(rate_limited_message(None).contains("next refresh"));
    }

    #[test]
    fn a_window_the_endpoint_omitted_is_absent_rather_than_zero() {
        let limits = normalize(&UsageResponse { five_hour: None, seven_day: None });
        assert!(limits.is_empty());
    }

    /// Checks the live endpoint against a real signed-in Claude Code installation.
    /// Ignored by default because it needs credentials and network access; run it with
    /// `cargo test claude_usage_endpoint -- --ignored --nocapture` after changing this
    /// adapter, to confirm the response still carries both windows on the scale the
    /// normalizer assumes.
    #[tokio::test]
    #[ignore = "requires a signed-in Claude Code installation and network access"]
    async fn claude_usage_endpoint_still_reports_both_windows() {
        let limits = read_windows().await.expect("read live Claude quota");
        println!("plan type: {:?}", plan_type());
        for limit in &limits {
            println!(
                "{} used {:?}% remaining {:?}% resets_at {:?}",
                limit.label, limit.used_percent, limit.remaining_percent, limit.resets_at
            );
            let used = limit.used_percent.expect("a window must report utilization");
            assert!((0.0..=100.0).contains(&used), "utilization must be a percentage");
        }
        assert!(!limits.is_empty(), "the endpoint must report at least one window");
    }

    #[test]
    fn an_expired_sign_in_is_recognised_before_any_request() {
        let credentials = |expires_at_ms| Credentials {
            access_token: String::new(),
            expires_at_ms,
            plan_type: None,
        };
        let now = jiff::Timestamp::now().as_millisecond();
        assert!(credentials(Some(now - 1_000)).is_expired());
        assert!(!credentials(Some(now + 60_000)).is_expired());
        assert!(
            !credentials(None).is_expired(),
            "an unknown expiry must not block a read"
        );
        // Claude Code records a live sign-in with a zero expiry, which is not an expiry.
        assert!(
            !credentials(Some(0)).is_expired(),
            "an unset expiry must not be read as the epoch"
        );
    }
}
