//! Backend preparation boundary; a frozen input record conveys no authority.

use async_trait::async_trait;
use event_application::CommittedMutation;
use identity_domain::{AuthenticatedIdentity, RequestId};
use release_domain::ContentHash;

use crate::{CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentId};

/// Original committed preparation receipt, separate from admission and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPreparationReceipt {
    /// Original Install command whose complete input was frozen.
    pub command: CommandIdentity,
    /// Exact immutable preparation fingerprint.
    pub input_hash: ContentHash,
    /// Real middleware request that first committed preparation.
    pub preparation_request_id: RequestId,
    /// Fresh event committed with the complete graph and its allowed audits.
    pub event: CommittedMutation,
}

/// Durable preparation result; no provider action or mount grant is included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPreparationAdmission {
    /// Complete revalidated original input, including server configuration.
    pub preparation: DeploymentExecutionPreparation,
    /// Original immutable receipt, stable across freshly authorized retries.
    pub receipt: ExecutionPreparationReceipt,
    /// Whether this attempt returned an already committed preparation.
    pub replayed: bool,
}

/// Backend-only authoritative whole-graph preparation and historical inspection.
///
/// Implementations receive policy/profile through trusted server composition,
/// recheck live permissions before replay, and commit all inputs before any IO.
/// Neither method accepts caller catalog JSON, policies, or worker receipt IDs.
#[async_trait]
pub trait DeploymentExecutionPreparationRepository: Send + Sync + 'static {
    /// Freezes every import under the exact original admitted Install command.
    ///
    /// # Errors
    /// Rejects denied current authority, malformed history, input/configuration
    /// drift, existing unprepared effects, or an atomic persistence failure.
    async fn prepare_execution(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        deployment: DeploymentId,
    ) -> Result<ExecutionPreparationAdmission, DeploymentError>;

    /// Reconstructs historical input under current project management only.
    ///
    /// The returned data is never reusable Install/source/provider authority.
    ///
    /// # Errors
    /// Rejects denied project access, unavailable deployment, or malformed data.
    async fn inspect_execution_preparation(
        &self,
        identity: &AuthenticatedIdentity,
        deployment: DeploymentId,
    ) -> Result<Option<DeploymentExecutionPreparation>, DeploymentError>;
}
