//! Application contracts for exact-run Git credential issuance.
//!
//! Runtime Git credentials are distinct from generic runtime credentials and
//! developer PATs. Only a verifier is durable; plaintext exists temporarily in
//! a host handoff envelope and the authenticated guest bootstrap stream.

mod credential;
mod error;
mod issuer;
mod repository;

#[cfg(test)]
mod tests;

/// Size of an opaque runtime Git credential.
pub const RUNTIME_GIT_CREDENTIAL_BYTES: usize = 32;

pub use credential::{RuntimeGitCredential, RuntimeGitCredentialHash};
pub use error::RuntimeGitAuthorityError;
pub use issuer::{IssuedRuntimeGitCredential, RuntimeGitCredentialIssuer};
pub use repository::{
    AuthenticatedRuntimeGitAuthority, RuntimeGitCredentialRepository, RuntimeGitHandoffStore,
    StoredRuntimeGitCredential,
};
