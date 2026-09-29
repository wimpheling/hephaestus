//! Local lifecycle management for the pinned forge-owned Zot registry.

mod configuration;
mod lifecycle;
#[cfg(test)]
mod tests;

pub use lifecycle::{challenge_ready, clean, initialize, running, show_logs, start, stop};

const CONFIG_TEMPLATE: &str = "deploy/zot/zot-config.json.tera";
const CONFIG_CONTAINER_PATH: &str = "/etc/zot/config.json";
const CERTIFICATE_CONTAINER_PATH: &str = "/etc/zot/verification.crt";
