#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // Claude Code starts this same executable as its status-line command and as its Stop
    // hook. Those runs read a payload, do one small thing, and end; neither must ever reach
    // the interface.
    if quotastation_lib::run_claude_hook() {
        return;
    }
    quotastation_lib::run();
}
