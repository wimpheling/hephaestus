use async_trait::async_trait;
use forge_domain::RepositoryId;
use identity_domain::{RequestId, UserId};
use release_domain::{UiInstallationGenerationId, ui_browser::UiBrowserSessionSecret};

use crate::UiBrowserSessionContext;

/// Smart-HTTP operation requested through the reserved UI Git path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRepositoryGitOperation {
    /// Read refs or objects.
    Read,
    /// Receive a write to the repository.
    Write,
}

impl UiRepositoryGitOperation {
    /// Returns the verifier operation spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// Typed repository Git authority returned by the browser verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiRepositoryGitAuthorization {
    /// Actual user selected by the live browser child session.
    pub actor_id: UserId,
    /// Exact repository bound to the UI installation.
    pub repository_id: RepositoryId,
    /// Effective approved access mode.
    pub access: release_domain::ui::UiRepositoryGitAccess,
    /// Complete child-session context used by the durable UI audit boundary.
    pub context: UiBrowserSessionContext,
}

/// Verified target projection for an installed UI context request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiBrowserTargetContext {
    /// Complete child-session context used by the durable UI audit boundary.
    pub context: UiBrowserSessionContext,
    /// Repository selected by the immutable repository-scoped installation.
    pub repository_id: RepositoryId,
}

/// Application-role port for generic installed-UI target discovery.
#[async_trait]
pub trait UiBrowserTargetContextProjection: Send + Sync {
    /// Rechecks the live child session, generation, installation, release, and
    /// route permissions before returning a repository target.
    async fn project_ui_target(
        &self,
        request_id: RequestId,
        session_secret: UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
    ) -> Result<UiBrowserTargetContext, UiTargetContextError>;
}

/// Redacted target-projection failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiTargetContextError {
    /// The child session is not authorized for a repository target.
    #[error("UI target context is unauthorized")]
    Unauthorized,
    /// The application verifier or connection was unavailable.
    #[error("UI target context is unavailable")]
    Unavailable,
}

/// Application-role port for reserved same-origin UI Git authorization.
#[async_trait]
pub trait UiBrowserRepositoryGitAuthorization: Send + Sync {
    /// Rechecks live session, installation, generation, release, and grants.
    async fn authorize_repository_git(
        &self,
        request_id: RequestId,
        session_secret: UiBrowserSessionSecret,
        expected_generation_id: UiInstallationGenerationId,
        repository_id: RepositoryId,
        operation: UiRepositoryGitOperation,
    ) -> Result<UiRepositoryGitAuthorization, UiGitAuthorizationError>;
}

/// Redacted repository Git authorization failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UiGitAuthorizationError {
    /// The live UI child or repository grants do not authorize this operation.
    #[error("UI repository Git request is unauthorized")]
    Unauthorized,
    /// The application verifier or connection was unavailable.
    #[error("UI repository Git authorization is unavailable")]
    Unavailable,
}
