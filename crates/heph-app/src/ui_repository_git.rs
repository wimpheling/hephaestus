//! Same-origin browser Git adapter for one installed UI repository.
//!
//! The adapter consumes the host-only child cookie at the UI boundary and
//! forwards only a verified human identity to the shared Git transport. It
//! never forwards browser cookies or bearer headers to `git-http`.

mod common;
mod handlers;
mod routes;
mod validation;

#[cfg(test)]
mod tests;

pub use common::UiRepositoryGitState;
pub use routes::router;
pub use validation::{error_response, parse_child_cookie, request_authority, require_same_origin};
