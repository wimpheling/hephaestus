//! Git authorization port types.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

use super::{AuthorizationDecision, AuthzError};

/// Git operation requiring repository authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepositoryOperation {
    /// Read refs and objects.
    Read,
    /// Write refs and objects.
    Write,
}

/// Provider-neutral authorization port for Git transport adapters.
#[async_trait]
pub trait GitRepositoryAuthorizer: Send + Sync + 'static {
    /// Authorizes an authenticated identity for one repository operation.
    async fn authorize_git(
        &self,
        repository_id: Uuid,
        operation: GitRepositoryOperation,
        identity: &AuthenticatedIdentity,
    ) -> Result<AuthorizationDecision, AuthzError>;
}
