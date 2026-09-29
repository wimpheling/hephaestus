use super::{ControlServiceError, ReviewRepositoryError};
use async_trait::async_trait;
use forge_domain::RepositoryId;
use review_domain::{ControlCommand, ReviewProposalId};
use runtime_types::RunId;
use std::path::PathBuf;

/// Result of idempotently processing one durable human control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The requested operation completed.
    Completed,
    /// A prior delivery already completed the operation.
    AlreadyCompleted,
    /// Authorization denied the operation and the request was closed.
    Denied,
    /// The requested operation was unsupported by the source and the request
    /// was closed without creating a new run.
    Rejected,
    /// The Git target moved and the proposal was marked conflicted.
    Conflicted,
}

/// Provider-neutral proposal data needed for trusted Git publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalProposal {
    /// Durable proposal identifier.
    pub id: ReviewProposalId,
    /// Repository containing the controlled refs.
    pub repository_id: RepositoryId,
    /// Run which produced the proposed result.
    pub run_id: RunId,
    /// Ref whose value is changed using compare-and-swap.
    pub target_ref: String,
    /// Exact target value from which the run started.
    pub input_commit: String,
    /// Exact proposed result commit.
    pub result_commit: String,
    /// Controlled host-written ref containing the proposed result.
    pub result_ref: String,
}

/// Result of the atomic approval preparation unit of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalPreparation {
    /// Authorization and durable claim succeeded; Git publication may run.
    Ready(ApprovalProposal),
    /// The command reached a durable terminal result without touching Git.
    Terminal(ControlOutcome),
}

/// Outcome of the external Git compare-and-swap effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDisposition {
    /// The target points at the proposed result.
    Approved,
    /// The target no longer points at the recorded input.
    Conflicted,
}

/// Atomic persistence boundaries used by review control orchestration.
#[async_trait]
pub trait ReviewRepository: Send + Sync {
    /// Executes a cancel, retry, or rejection in one durable transaction.
    async fn execute_control(
        &self,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ReviewRepositoryError>;

    /// Authorizes and durably claims an approval in one transaction.
    async fn prepare_approval(
        &self,
        command: &ControlCommand,
    ) -> Result<ApprovalPreparation, ReviewRepositoryError>;

    /// Finalizes the durable result after the external Git effect.
    async fn finalize_approval(
        &self,
        command: &ControlCommand,
        proposal: &ApprovalProposal,
        disposition: ApprovalDisposition,
    ) -> Result<ControlOutcome, ReviewRepositoryError>;
}

/// Resolves canonical repository paths without exposing a storage provider.
#[async_trait]
pub trait RepositoryLocator: Send + Sync {
    /// Validates and resolves a repository's canonical bare-Git path.
    async fn locate(&self, repository_id: RepositoryId) -> Result<PathBuf, String>;
}

/// External Git publication boundary used by the control orchestrator.
#[async_trait]
pub trait ReviewGit: Send + Sync {
    /// Validates provenance and publishes the result with compare-and-swap.
    async fn publish(
        &self,
        proposal: &ApprovalProposal,
    ) -> Result<ApprovalDisposition, ControlServiceError>;
}
