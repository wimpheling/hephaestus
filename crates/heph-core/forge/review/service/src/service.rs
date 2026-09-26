use super::{
    ApprovalPreparation, ControlOutcome, ControlServiceError, ReviewGit, ReviewRepository,
};
use review_domain::{ControlCommand, ControlKind};
use std::sync::Arc;

/// Trusted host service for browser-originated run and review commands.
#[derive(Clone)]
pub struct ReviewControlService {
    repository: Arc<dyn ReviewRepository>,
    git: Arc<dyn ReviewGit>,
}

impl ReviewControlService {
    /// Creates a service using trusted Git publication over a path locator.
    #[must_use]
    pub fn new(
        repository: Arc<dyn ReviewRepository>,
        locator: Arc<dyn super::RepositoryLocator>,
    ) -> Self {
        Self::with_git(
            repository,
            Arc::new(super::GitReviewPublisher::new(locator)),
        )
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
