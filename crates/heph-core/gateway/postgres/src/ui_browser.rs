//! Worker-side `PostgreSQL` authority for the trusted UI gateway admission port.
//!
//! The adapter uses migration 0090's canonical verifier after selecting the
//! current gateway binding and reading only the immutable child digest.

mod admission;
mod authorization;
mod routing;

pub use admission::accept_ui_invocation;
