use super::runtime::RunLaunchAuthorizer;
use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use uuid::Uuid;
use vm_trait::RuntimeAuthorityBootstrap;
use workspace_domain::{PreparedRuntimeGitWorkspace, RuntimeGitWorkspaceManager, WorkspaceError};

/// Lifecycle boundary for one exact run's generic runtime authority session.
///
/// The concrete adapter owns authorization-snapshot resolution, durable
/// hash-only session issuance, and the host-only encrypted handoff. The
/// orchestrator deliberately never handles bearer bytes.
#[async_trait]
pub trait RunAuthorityManager: Send + Sync + 'static {
    /// Resolves the immutable capability ceiling and stages one exact session
    /// for bootstrap delivery.
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError>;
    /// Rechecks live authority immediately before VM provisioning.
    async fn reauthorize(&self, run: &Run) -> Result<(), RunAuthorityError>;
    /// Records a guest acknowledgement already matched to the exact staged
    /// session and issuance generation by the orchestrator.
    async fn acknowledge(
        &self,
        run: &Run,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), RunAuthorityError>;
    /// Revokes the session and destroys any remaining handoff after the guest
    /// has been destroyed.
    async fn revoke_after_guest(&self, run_id: RunId) -> Result<(), RunAuthorityError>;
    /// Reconciles expired sessions and orphan host-only handoffs.
    async fn recover(&self) -> Result<usize, RunAuthorityError>;
}

/// Sensitive authority staged for provider bootstrap delivery.
#[derive(Debug, Default)]
pub struct PreparedRunAuthority {
    /// Optional one-run bootstrap payload. Absence means the configured
    /// manager intentionally issues no generic runtime session.
    pub bootstrap: Option<RuntimeAuthorityBootstrap>,
}

#[derive(Debug)]
pub(super) struct DisabledRunAuthorityManager;

#[async_trait]
impl RunAuthorityManager for DisabledRunAuthorityManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        Ok(PreparedRunAuthority::default())
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunAuthorityError> {
        Ok(())
    }

    async fn acknowledge(
        &self,
        _run: &Run,
        _session_id: Uuid,
        _generation: u64,
    ) -> Result<(), RunAuthorityError> {
        Ok(())
    }

    async fn revoke_after_guest(&self, _run_id: RunId) -> Result<(), RunAuthorityError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        Ok(0)
    }
}

#[derive(Debug, Default)]
pub(super) struct DisabledRuntimeGitWorkspaceManager;

#[async_trait]
impl RuntimeGitWorkspaceManager for DisabledRuntimeGitWorkspaceManager {
    async fn prepare_runtime_git(
        &self,
        _run: &Run,
    ) -> Result<Option<PreparedRuntimeGitWorkspace>, WorkspaceError> {
        Ok(None)
    }

    async fn abandon_runtime_git(&self, _run_id: RunId) -> Result<(), WorkspaceError> {
        Ok(())
    }

    async fn recover_runtime_git(&self) -> Result<usize, WorkspaceError> {
        Ok(0)
    }
}

/// Redacted generic runtime-authority lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("run authority operation failed: {message}")]
pub struct RunAuthorityError {
    message: String,
}

impl RunAuthorityError {
    /// Creates a non-disclosing authority lifecycle failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct DisabledRunLaunchAuthorizer;

#[async_trait]
impl RunLaunchAuthorizer for DisabledRunLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        Ok(())
    }
}

/// Redacted live launch-authorization failure.
#[derive(Debug, thiserror::Error)]
#[error("run launch authorization failed: {message}")]
pub struct RunAuthorizationError {
    message: String,
}

impl RunAuthorizationError {
    /// Creates a redacted authorization failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
