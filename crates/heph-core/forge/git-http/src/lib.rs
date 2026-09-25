//! Authorized, bounded, streaming Git smart-HTTP transport.

pub mod receive_hook;
pub mod receive_policy;

#[path = "git_http/api.rs"]
mod api;
#[path = "git_http/auth.rs"]
mod auth;
#[path = "git_http/backend.rs"]
mod backend;
#[path = "git_http/errors.rs"]
mod errors;
#[path = "git_http/execution.rs"]
mod execution;
#[path = "git_http/principal.rs"]
mod principal;
#[path = "git_http/refs.rs"]
mod refs;
#[path = "git_http/service.rs"]
mod service;

pub use api::{
    AuthorizationRequest, GitAuthenticator, GitAuthorizer, GitOperation, HumanPrincipal, Principal,
    RuntimePrincipal,
};
pub use auth::{
    CompositeGitAuthenticator, OidcGitAuthenticator, PostgresGitAuthorizer,
    RuntimeGitHttpAuthenticator,
};
pub use errors::{AuthenticationError, AuthorizationError, GitHttpError};
pub use execution::execute_authenticated_human;
pub use service::{
    AuthenticatedHumanGitEndpoint, AuthenticatedHumanGitRequest, GitHttpLimits, GitHttpService,
};

#[cfg(test)]
#[path = "git_http/tests.rs"]
mod tests;
