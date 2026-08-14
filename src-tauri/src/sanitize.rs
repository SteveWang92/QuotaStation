//! Diagnostics reach the renderer and the local database, so provider and
//! storage failures must never carry filesystem layout or account details.

const MAX_LEN: usize = 220;

/// Reduce a failure to a single redacted line suitable for display and storage.
pub fn sanitize_error(error: &str, fallback: &str) -> String {
    let line = error.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        return fallback.to_string();
    }
    truncate(&redact_paths(line))
}

fn truncate(line: &str) -> String {
    match line.char_indices().nth(MAX_LEN) {
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
    fn falls_back_on_empty_input() {
        assert_eq!(sanitize_error("   ", "Storage failed"), "Storage failed");
    }

    #[test]
    fn truncates_on_a_character_boundary() {
        let message = sanitize_error(&"é".repeat(400), "Storage failed");
        assert_eq!(message.chars().count(), 221);
    }
}
