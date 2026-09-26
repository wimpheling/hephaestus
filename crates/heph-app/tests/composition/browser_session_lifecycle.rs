#![cfg(feature = "test-fixtures")]

//! Real daemon transport proof for the browser-session lifecycle.
//!
//! The sibling support fixture owns the disposable application configuration
//! and database/NATS cleanup boundary without changing the golden test or
//! introducing a second production fixture implementation.

#[path = "../support/browser_session_fixture.rs"]
mod app_fixture;

// The integration crate is test-only; keep its transport and database probes
// outside the production architecture surface while retaining the composition
// boundary for generated Connect types.
#[cfg(test)]
#[path = "browser_session_lifecycle/transport.rs"]
mod browser_session_transport;
