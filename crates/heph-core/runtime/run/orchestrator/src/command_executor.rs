//! Durable start/cancel boundary; profile admission belongs to the implementation.

use async_trait::async_trait;
use run_domain::{CancelRun, StartRun};

use crate::{
    OrchestratorError, RepositoryError, RunCleanupWakeProgress, RunCleanupWakeRequest,
    RunOrchestrator,
};
use std::sync::Arc;

/// Executes durable commands after implementation-specific admission checks.
///
/// A routed implementation must prove the exact persisted producer/profile and
/// configured owner before selecting an orchestrator. This port supplies no
/// admission, physical ownership or authority to choose a profile from a payload.
#[async_trait]
pub trait RunCommandExecutor: Send + Sync + 'static {
    /// Wakes exact already-cancelled owning cleanup, creating no command or IO grant.
    ///
    /// # Errors
    /// Default/unknown adapters are unsupported; owning failures remain held.
    async fn wake_cleanup(
        self: Arc<Self>,
        _request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        Err(RepositoryError::InvalidData("owning cleanup wake is unsupported").into())
    }

    /// Executes one exact start command and preserves durable idempotency.
    ///
    /// # Errors
    /// Returns the orchestration/admission failure without acknowledging delivery.
    async fn start_run(&self, command: &StartRun) -> Result<(), OrchestratorError>;

    /// Executes cancellation without waiting behind an unrelated start future.
    ///
    /// # Errors
    /// Returns the persistence or shutdown failure without acknowledging delivery.
    async fn cancel_run(&self, command: &CancelRun) -> Result<bool, OrchestratorError>;
}

#[async_trait]
impl RunCommandExecutor for RunOrchestrator {
    async fn wake_cleanup(
        self: Arc<Self>,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        Self::wake_cleanup(self, request).await
    }

    async fn start_run(&self, command: &StartRun) -> Result<(), OrchestratorError> {
        Self::start_run(self, command).await.map(|_| ())
    }

    async fn cancel_run(&self, command: &CancelRun) -> Result<bool, OrchestratorError> {
        Self::cancel_run(self, command).await
    }
}
