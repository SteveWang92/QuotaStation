#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // Claude Code starts this same executable as its status-line command. That run reads a
    // payload, prints a line, and ends; it must never reach the interface.
    if quotastation_lib::run_claude_status_line() {
        return;
    }
    quotastation_lib::run();
}
