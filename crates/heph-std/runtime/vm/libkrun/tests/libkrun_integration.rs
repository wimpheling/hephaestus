//! Opt-in hardware integration tests for the Fedora libkrun backend.

#[path = "libkrun_integration/boot.rs"]
mod boot;
#[path = "libkrun_integration/git_bridge.rs"]
mod git_bridge;
#[path = "libkrun_integration/support/mod.rs"]
mod support;

const ENABLE_FLAG: &str = "HEPHAESTUS_LIBKRUN_INTEGRATION";
static INTEGRATION_TEST_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
    std::sync::OnceLock::new();
