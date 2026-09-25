//! The narrow UI-origin bootstrap HTTP boundary.
//!
//! This module is intentionally limited to `/_heph/bootstrap`. It resolves
//! the exact generation host before inspecting the handoff, exchanges the
//! one-time bearer through the release service port, and returns only a safe
//! route. Static files and managed/API forwarding belong to later handlers.
//!
//! Intended home: `crates/hephaestus-app/src/ui_bootstrap.rs`, after the host
//! and read-port candidates are integrated into `release-service`.

mod config;
mod helpers;
mod routes;

#[cfg(test)]
mod tests;

// These re-exports preserve the existing public module API while the
// implementation lives in the configuration child module.
#[allow(unused_imports)]
pub use config::{
    DEFAULT_BOOTSTRAP_CONCURRENCY, DEFAULT_BOOTSTRAP_DEADLINE, MAX_HANDOFF_BODY_BYTES,
    UiBootstrapConfig, UiBootstrapConfigError, UiBootstrapState,
};
pub use routes::router;
