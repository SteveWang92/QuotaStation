#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // The NSIS uninstaller asks the installed executable to remove the two Claude Code
    // integrations before it deletes the binary. This path starts no Tauri window or
    // provider reader, and upgrades never invoke it.
    if let Some(exit_code) = quotastation_lib::run_uninstall_cleanup() {
        std::process::exit(exit_code);
    }
    // Claude Code starts this same executable as its status-line command and as its Stop
    // hook. Those runs read a payload, do one small thing, and end; neither must ever reach
    // the interface.
    if quotastation_lib::run_claude_hook() {
        return;
    }
    quotastation_lib::run();
}
