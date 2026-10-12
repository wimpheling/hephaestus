#[path = "basic.rs"]
mod basic;
mod cleanup_emptiness;
#[path = "launch.rs"]
mod launch;
#[path = "lifecycle.rs"]
mod lifecycle;
mod managed_configuration;
mod ownership;
mod ownership_cleanup_failure;
mod ownership_provision;
mod reap_destroy;
#[path = "service.rs"]
mod service;
mod stop_start;
mod supervision;
#[path = "support.rs"]
mod support;
