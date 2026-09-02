//! Diagnostics reach the renderer and the local database, so provider and
//! storage failures must never carry filesystem layout or account details.

const MAX_LEN: usize = 220;
/// The activity log's allowance. It is read after the fact, by someone who has only the
/// file to go on, so a line there can afford more than one a dialog has to fit — and a
/// stack trace cut off at its first frame is a line that answers nothing.
const MAX_LOG_LEN: usize = 800;

/// Reduce a failure to a single redacted line suitable for display and storage.
pub fn sanitize_error(error: &str, fallback: &str) -> String {
    let line = error.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return fallback.to_string();
    }
    truncate(&redact_paths(line))
}

/// Reduce an activity-log entry to one redacted line.
///
/// Unlike a displayed failure, the whole message is kept rather than its first line: a
/// renderer stack or a multi-line provider error is exactly what the log exists to carry,
/// so the breaks become separators instead of a reason to throw the rest away.
pub fn sanitize_log(message: &str) -> String {
    let joined = message.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>();
    truncate_to(&redact_paths(&joined.join(" | ")), MAX_LOG_LEN)
}

fn truncate(line: &str) -> String {
    truncate_to(line, MAX_LEN)
}

fn truncate_to(line: &str, limit: usize) -> String {
    match line.char_indices().nth(limit) {
        Some((index, _)) => format!("{}…", &line[..index]),
        None => line.to_string(),
    }
}

fn redact_paths(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for piece in line.split_inclusive(char::is_whitespace) {
        let token = piece.trim_end();
        if looks_like_path(token) {
            out.push_str("<path>");
        } else {
            out.push_str(token);
        }
        out.push_str(&piece[token.len()..]);
    }
    out
}

fn looks_like_path(token: &str) -> bool {
    let token = token.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',' | ';' | ':'));
    if token.len() < 3 {
        return false;
    }
    let bytes = token.as_bytes();
    let windows_drive_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    windows_drive_path
        || token.contains('\\')
        || token.contains("://")
        || token.starts_with("~/")
        || (token.starts_with('/') && token[1..].contains('/'))
}

#[cfg(test)]
mod tests {
    use super::sanitize_error;

    #[test]
    fn redacts_local_paths() {
        let message = sanitize_error(
            "error returned from database: unable to open C:\\Users\\example\\AppData\\quotastation.db",
            "Storage failed",
        );
        assert_eq!(message, "error returned from database: unable to open <path>");
    }

    #[test]
    fn redacts_windows_paths_with_forward_slashes() {
        let message = sanitize_error(
            "unable to read C:/Users/example/.claude/settings.json",
            "Provider failed",
        );
        assert_eq!(message, "unable to read <path>");
    }

    #[test]
    fn redacts_connection_urls_and_keeps_first_line_only() {
        let message = sanitize_error(
            "sqlite://C:\\Users\\example\\quotastation.db: locked\nsecond line",
            "Storage failed",
        );
        assert_eq!(message, "<path> locked");
    }

    #[test]
    fn a_log_entry_keeps_every_line_it_was_given() {
        let message = super::sanitize_log(
            "renderer failed
  at render
  at mount",
        );
        assert_eq!(message, "renderer failed | at render | at mount");
    }

    #[test]
    fn falls_back_on_empty_input() {
        assert_eq!(sanitize_error("   ", "Storage failed"), "Storage failed");
    }

    #[test]
    fn truncates_on_a_character_boundary() {
        let message = sanitize_error(&"é".repeat(400), "Storage failed");
        assert_eq!(message.chars().count(), 221);
    }
}
