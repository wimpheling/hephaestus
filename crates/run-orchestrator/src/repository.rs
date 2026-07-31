use async_trait::async_trait;
use run_domain::{CancelRun, InvalidTransition, Run, RunState, StartRun};
use runtime_types::{LeaseId, RunId, VolumeId};
use serde_json::Value;
use time::OffsetDateTime;
use vm_trait::VmExit;

/// Result of idempotently creating a run.
#[derive(Debug, Clone)]
pub struct CreateRunResult {
    /// Durable run.
    pub run: Run,
    /// Whether this call inserted the run.
    pub created: bool,
}

/// Durable representation of one provider event.
#[derive(Debug, Clone)]
pub struct StoredVmEvent {
    /// Stable event type.
    pub event_type: String,
    /// Structured event body.
    pub payload: Value,
    /// Time the orchestrator observed the event.
    pub occurred_at: OffsetDateTime,
}

/// Durable run repository failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RepositoryError {
    /// The requested run does not exist.
    #[error("run {0} was not found")]
    NotFound(RunId),
    /// The requested state transition is invalid.
    #[error(transparent)]
    InvalidTransition(#[from] InvalidTransition),
    /// Persistent storage failed.
    #[error("run repository operation failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Stored data violates the domain model.
    #[error("invalid stored run data: {0}")]
    InvalidData(&'static str),
}

/// Persistence boundary required by the orchestrator.
#[async_trait]
pub trait RunRepository: Send + Sync + 'static {
    /// Creates a queued run and command-inbox record idempotently.
    async fn create_run(&self, command: &StartRun) -> Result<CreateRunResult, RepositoryError>;
    /// Loads one run.
    async fn get(&self, run_id: RunId) -> Result<Run, RepositoryError>;
    /// Binds the durable VM identifier and any state-volume resources.
    async fn bind_resources(
        &self,
        run_id: RunId,
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError>;
    /// Applies one valid state transition and records its bounded run event.
    async fn transition(
        &self,
        run_id: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError>;
    /// Persists one best-effort VM event.
    async fn append_vm_event(
        &self,
        run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError>;
    /// Records an idempotent cancellation command.
    async fn request_cancel(&self, command: &CancelRun) -> Result<bool, RepositoryError>;
    /// Returns non-cleaned runs that may require restart reconciliation.
    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError>;
}
