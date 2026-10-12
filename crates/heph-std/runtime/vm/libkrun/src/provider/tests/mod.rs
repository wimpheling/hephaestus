#[path = "basic.rs"]
mod basic;
mod child_custody;
mod cleanup_emptiness;
mod generation_drain;
#[path = "launch.rs"]
mod launch;
mod launch_custody;
#[path = "lifecycle.rs"]
mod lifecycle;
mod lifecycle_cancellation;
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

mod admission_inventory;
