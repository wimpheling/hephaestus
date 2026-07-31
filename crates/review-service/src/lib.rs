//! Provider-neutral review control orchestration and trusted Git publication.

use async_trait::async_trait;
use forge_domain::{CommitSha, GitRef, RepositoryId};
use review_domain::{
    CONTROL_EXECUTE_SUBJECT, ControlCommand, ControlKind, ControlRequestId, ReviewProposalId,
};
use runtime_types::RunId;
use std::{path::Path, path::PathBuf, process::Stdio, sync::Arc};
use tokio::process::Command;

mod command_transport;

pub use command_transport::{
    NatsControlHandler, ReviewOutboxPublishError, ReviewOutboxPublisher, ReviewOutboxRecord,
    ReviewOutboxStore, ReviewOutboxStoreError,
};

/// Result of idempotently processing one durable human control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The requested operation completed.
    Completed,
    /// A prior delivery already completed the operation.
    AlreadyCompleted,
    /// Authorization denied the operation and the request was closed.
    Denied,
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

/// Provider-neutral persistence failure for review units of work.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReviewRepositoryError {
    /// A delivery did not match its authoritative persisted command.
    #[error("control delivery does not match its authoritative row")]
    DeliveryMismatch,
    /// The durable control request does not exist.
    #[error("control request {0} does not exist")]
    MissingControl(ControlRequestId),
    /// The review proposal does not exist.
    #[error("review proposal {0} does not exist")]
    MissingProposal(ReviewProposalId),
    /// The source run did not originate from an accepted forge request.
    #[error("run {0} has no forge run request")]
    MissingRunRequest(RunId),
    /// The proposal is no longer actionable.
    #[error("review proposal is closed in state {0}")]
    ProposalClosed(String),
    /// A persistence or authorization provider failed.
    #[error("review repository operation failed: {0}")]
    Infrastructure(String),
}

/// Atomic persistence boundaries used by review control orchestration.
///
/// Implementations must commit each method before returning. In particular,
/// [`Self::prepare_approval`] must not retain a transaction while the caller
/// performs Git I/O, and [`Self::finalize_approval`] must be independently
/// retryable after the Git effect has completed.
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

/// Trusted host implementation of review result publication.
#[derive(Clone)]
pub struct GitReviewPublisher {
    locator: Arc<dyn RepositoryLocator>,
}

impl GitReviewPublisher {
    /// Creates a publisher over canonical repository path resolution.
    #[must_use]
    pub fn new(locator: Arc<dyn RepositoryLocator>) -> Self {
        Self { locator }
    }
}

#[async_trait]
impl ReviewGit for GitReviewPublisher {
    async fn publish(
        &self,
        proposal: &ApprovalProposal,
    ) -> Result<ApprovalDisposition, ControlServiceError> {
        let repository = self
            .locator
            .locate(proposal.repository_id)
            .await
            .map_err(ControlServiceError::Storage)?;
        let target_ref = GitRef::parse(proposal.target_ref.clone())?;
        let result_ref = GitRef::parse(proposal.result_ref.clone())?;
        let input = CommitSha::parse(proposal.input_commit.clone())?;
        let result = CommitSha::parse(proposal.result_commit.clone())?;
        validate_result_provenance(&repository, &result_ref, &result, &input).await?;
        let current = resolve_ref(&repository, &target_ref).await?;
        if current.as_ref() == Some(&result) {
            return Ok(ApprovalDisposition::Approved);
        }
        if current.as_ref() != Some(&input) {
            return Ok(ApprovalDisposition::Conflicted);
        }
        cas_update_ref(&repository, &target_ref, &result, &input).await?;
        Ok(ApprovalDisposition::Approved)
    }
}

/// Trusted host service for browser-originated run and review commands.
#[derive(Clone)]
pub struct ReviewControlService {
    repository: Arc<dyn ReviewRepository>,
    git: Arc<dyn ReviewGit>,
}

impl ReviewControlService {
    /// Creates a service using trusted Git publication over a path locator.
    #[must_use]
    pub fn new(repository: Arc<dyn ReviewRepository>, locator: Arc<dyn RepositoryLocator>) -> Self {
        Self::with_git(repository, Arc::new(GitReviewPublisher::new(locator)))
    }

    /// Creates a service from explicit persistence and Git boundaries.
    #[must_use]
    pub fn with_git(repository: Arc<dyn ReviewRepository>, git: Arc<dyn ReviewGit>) -> Self {
        Self { repository, git }
    }

    /// Processes one outbox-derived command with durable idempotency.
    ///
    /// Approval preparation commits before Git validation and compare-and-swap.
    /// Finalization then runs as a separate retryable transaction.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid delivery, persistence failure, or Git
    /// validation/publication failure. Retrying the same command is safe.
    pub async fn execute(
        &self,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ControlServiceError> {
        command.validate()?;
        if command.kind != ControlKind::ApproveResult {
            return self
                .repository
                .execute_control(command)
                .await
                .map_err(Into::into);
        }
        match self.repository.prepare_approval(command).await? {
            ApprovalPreparation::Terminal(outcome) => Ok(outcome),
            ApprovalPreparation::Ready(proposal) => {
                let disposition = self.git.publish(&proposal).await?;
                self.repository
                    .finalize_approval(command, &proposal, disposition)
                    .await
                    .map_err(Into::into)
            }
        }
    }
}

async fn validate_result_provenance(
    repository: &Path,
    result_ref: &GitRef,
    result: &CommitSha,
    input: &CommitSha,
) -> Result<(), ControlServiceError> {
    let published = resolve_ref(repository, result_ref).await?;
    if published.as_ref() != Some(result) {
        return Err(ControlServiceError::InvalidResultProvenance(String::from(
            "controlled result ref does not point at the recorded result",
        )));
    }
    let parent = git_text(repository, &["rev-parse", &format!("{}^", result.as_str())]).await?;
    if parent != input.as_str() {
        return Err(ControlServiceError::InvalidResultProvenance(String::from(
            "result commit parent is not the exact input commit",
        )));
    }
    Ok(())
}

async fn resolve_ref(
    repository: &Path,
    git_ref: &GitRef,
) -> Result<Option<CommitSha>, ControlServiceError> {
    let output = git_output(repository, &["rev-parse", "--verify", git_ref.as_str()]).await?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Ok(Some(CommitSha::parse(value)?));
    }
    Ok(None)
}

async fn cas_update_ref(
    repository: &Path,
    target: &GitRef,
    result: &CommitSha,
    input: &CommitSha,
) -> Result<(), ControlServiceError> {
    let output = git_output(
        repository,
        &[
            "update-ref",
            target.as_str(),
            result.as_str(),
            input.as_str(),
        ],
    )
    .await?;
    if !output.status.success() {
        return Err(ControlServiceError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(())
}

async fn git_text(repository: &Path, arguments: &[&str]) -> Result<String, ControlServiceError> {
    let output = git_output(repository, arguments).await?;
    if !output.status.success() {
        return Err(ControlServiceError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn git_output(
    repository: &Path,
    arguments: &[&str],
) -> Result<std::process::Output, ControlServiceError> {
    Command::new("git")
        .arg("--git-dir")
        .arg(repository)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(ControlServiceError::Io)
}

/// Durable control processing failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlServiceError {
    /// Command targets did not match its operation.
    #[error(transparent)]
    InvalidCommand(#[from] review_domain::InvalidControlCommand),
    /// Persistence or authorization failed.
    #[error(transparent)]
    Repository(#[from] ReviewRepositoryError),
    /// Recorded result provenance failed trusted host validation.
    #[error("invalid result provenance: {0}")]
    InvalidResultProvenance(String),
    /// A Git value stored in persistence was invalid.
    #[error(transparent)]
    GitValue(#[from] forge_domain::GitValueError),
    /// Canonical repository resolution failed.
    #[error("canonical repository resolution failed: {0}")]
    Storage(String),
    /// Git process launch failed.
    #[error("Git process failed: {0}")]
    Io(#[source] std::io::Error),
    /// Git rejected an operation.
    #[error("Git operation failed: {0}")]
    Git(String),
}

/// Control delivery failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlHandlingError {
    /// Delivery used an unsupported subject.
    #[error("unsupported control subject {0}")]
    UnknownSubject(String),
    /// Delivery payload was not a valid command.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// Durable processing failed.
    #[error(transparent)]
    Service(#[from] ControlServiceError),
    /// `JetStream` did not confirm acknowledgement.
    #[error("control acknowledgement failed: {0}")]
    Acknowledgement(String),
}
