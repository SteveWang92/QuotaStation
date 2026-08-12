mod api;
mod history;
mod session;

use anyhow::Result;

use crate::domain::{CrossCheck, LiveSnapshot};

pub use history::read_history;

/// Claude quota, from the local session logs, optionally corrected by Anthropic's own
/// usage endpoint.
///
/// The logs are always the source: they need no credential, cannot be rate limited, and
/// give the window currently running and when it ends. They cannot give how much of it is
/// left, which is the one thing the endpoint adds. So when the cross-check is switched on
/// and answers, its windows replace the derived one; when it does not answer, the derived
/// window stands and the reason is carried alongside rather than replacing the reading.
pub async fn read_live(cross_check: bool) -> Result<LiveSnapshot> {
    let mut snapshot = session::read_live(api::plan_type()).await?;
    if !cross_check {
        return Ok(snapshot);
    }
    match api::read_windows().await {
        Ok(limits) if !limits.is_empty() => {
            snapshot.limits = limits;
            snapshot.cross_check = CrossCheck::Confirmed;
        }
        Ok(_) => {
            snapshot.cross_check =
                CrossCheck::Failed("The usage endpoint reported no quota windows.".to_string());
        }
        Err(error) => snapshot.cross_check = CrossCheck::Failed(error.to_string()),
    }
    Ok(snapshot)
}
