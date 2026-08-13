//! The Claude plan recorded next to Claude Code's sign-in.
//!
//! Reading it involves no request and nothing leaving this machine: the plan is stored
//! alongside the token, so naming the tier costs neither a network call nor the account
//! identifiers that `~/.claude.json` also holds.
//!
//! The token itself is never read here. QuotaStation previously presented it to Anthropic's
//! OAuth usage endpoint as a second quota source; that endpoint rate-limits an account as a
//! whole and is already being read by Claude Code's own usage display, so in practice it
//! answered `429` and nothing else. The percentages now come from Claude Code's status line,
//! which costs no credential at all, and this module reads only the plan name.

use std::path::Path;

use super::claude_home;

/// The plan recorded next to the sign-in token, when there is one.
pub fn plan_type() -> Option<String> {
    let path = claude_home()?.join(".credentials.json");
    read_plan_type(&path)
}

fn read_plan_type(path: &Path) -> Option<String> {
    // Deliberately unwrapped and un-logged: an error carrying the credentials location
    // would end up in the database and the diagnostics panel.
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let oauth = value.get("claudeAiOauth")?;
    ["subscriptionType", "rateLimitTier"].into_iter().find_map(|field| {
        oauth
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The credential fixtures below carry no real token: only the plan fields are read,
    /// and the file has to exist somewhere for the reader to open it.
    fn write(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("quotastation-{name}-credentials.json"));
        std::fs::write(&path, content).expect("write credentials fixture");
        path
    }

    #[test]
    fn the_subscription_names_the_plan() {
        let path = write(
            "subscription",
            r#"{"claudeAiOauth":{"accessToken":"fixture","subscriptionType":"max"}}"#,
        );
        assert_eq!(read_plan_type(&path).as_deref(), Some("max"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_rate_limit_tier_answers_when_no_subscription_is_recorded() {
        let path = write(
            "tier",
            r#"{"claudeAiOauth":{"accessToken":"fixture","rateLimitTier":"default_max_20x"}}"#,
        );
        assert_eq!(read_plan_type(&path).as_deref(), Some("default_max_20x"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_sign_in_without_a_plan_leaves_the_tier_unknown() {
        let path = write("plainly", r#"{"claudeAiOauth":{"accessToken":"fixture"}}"#);
        assert_eq!(read_plan_type(&path), None);
        assert_eq!(read_plan_type(&std::env::temp_dir().join("quotastation-absent.json")), None);
        let _ = std::fs::remove_file(&path);
    }
}
