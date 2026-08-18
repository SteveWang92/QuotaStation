fn main() {
    let lock_path = "../vendor/ccusage/flake.lock";
    println!("cargo:rerun-if-changed={lock_path}");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    let lock: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(lock_path).expect("read vendored ccusage flake.lock"),
    )
    .expect("parse vendored ccusage flake.lock");
    let revision = lock
        .pointer("/nodes/litellm/locked/rev")
        .and_then(serde_json::Value::as_str)
        .expect("ccusage flake.lock must pin LiteLLM");
    println!("cargo:rustc-env=QUOTASTATION_PRICING_REVISION={revision}");
    tauri_build::build()
}
