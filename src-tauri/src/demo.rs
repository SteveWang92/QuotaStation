//! A QuotaStation that shows made-up data, for a screenshot.
//!
//! Every surface draws the snapshot the core restored from the database at startup, so a
//! database of fictional readings is enough to fill the tray, the quick panel, the taskbar
//! widget and the dashboard at once — nothing in the renderer needs to know it is looking
//! at a demonstration. What that leaves is the acquisition the core would otherwise start
//! immediately: a refresh, the session watcher and the periodic polls would replace the
//! seeded readings with this machine's real usage within seconds of the window opening, and
//! the screenshot would be of somebody's actual account. So a demo start reads no provider
//! at all.
//!
//! It also keeps its own database and settings file beside the real pair, because a demo
//! run must never write into the history the maintainer is actually keeping.
//!
//! The data itself is not here and is not in the shipped application: `cargo run --example
//! seed_demo` writes it. See `docs/development.md`.

/// Starts QuotaStation against the demonstration database, with no provider access.
pub const DEMO_ARG: &str = "--demo";

/// The demonstration database, beside `quotastation.db` in the application data directory.
pub const DATABASE_FILE: &str = "quotastation-demo.db";

/// The demonstration settings, beside `settings.json`. A demo instance must not read the
/// real one: it carries this machine's device name, which is exactly the kind of detail a
/// published screenshot should not show.
pub const SETTINGS_FILE: &str = "settings-demo.json";

/// Whether this process was asked to run as a demonstration.
pub fn requested() -> bool {
    std::env::args_os().any(|argument| argument == DEMO_ARG)
}
