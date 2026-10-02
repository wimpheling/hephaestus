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
    /// Captures the immutable runtime Git target before materialization.
    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError>;
    /// Loads one run.
    async fn get(&self, run_id: RunId) -> Result<Run, RepositoryError>;
    /// Binds the durable VM identifier and any state-volume resources.
    async fn bind_resources(
        &self,
        run_id: RunId,
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        lease_fencing_token: Option<i64>,
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

/// Worker-only persistence boundary for canonical complete-set cleanup.
///
/// Existing run repositories are not implicitly cleanup implementations. An
/// adapter must enforce acquisition closure, exact target identity, and trusted
/// provider observations; public domain values alone establish no cleanup proof.
#[async_trait]
pub trait RunCleanupRepository: Send + Sync + 'static {
    /// Persists an exact planned VM identity before any provisioning IO.
    ///
    /// Trusted configured provider ownership supplies the host scope. Replays
    /// must compare scope, VM identity, and exact run/revision. An adapter must
    /// reject replacement, closed acquisition, and fabricated historical IDs.
    async fn bind_vm_before_provision(
        &self,
        run_id: RunId,
        instance_id: runtime_types::AgentInstanceId,
        revision_id: runtime_types::AgentInstanceRevisionId,
        host: &run_domain::RunCleanupHostId,
        vm_id: &vm_trait::VmId,
    ) -> Result<(), RepositoryError>;

    /// Closes further acquisition under the run lock and snapshots every lease.
    ///
    /// Returns the same immutable generation/target on retry, including an
    /// explicit empty set or unresolved historical VM identity. No IO belongs
    /// in this transaction, and no in-memory attachment is a source of truth.
    async fn begin_cleanup(
        &self,
        run_id: RunId,
    ) -> Result<run_domain::RunCleanupTarget, RepositoryError>;

    /// Loads an immutable observation before repeating provider destruction IO.
    ///
    /// The worker adapter must check configured provider ownership even for an
    /// empty lease set. Absence of a receipt is not evidence of absent resources.
    async fn recorded_cleanup_receipt(
        &self,
        run_id: RunId,
    ) -> Result<Option<run_domain::RunCleanupReceipt>, RepositoryError>;

    /// Persists a worker-verified exact VM destruction or authoritative absence.
    ///
    /// A DTO does not prove the observation: the trusted adapter must verify the
    /// worker boundary and matching persisted generation/full fence-set digest.
    async fn record_cleanup_receipt(
        &self,
        receipt: &run_domain::RunCleanupReceipt,
    ) -> Result<(), RepositoryError>;

    /// Consumes the persisted exact receipt and releases the complete fence set.
    ///
    /// One transaction verifies acquisition remains closed, matches every lease
    /// and fence, releases the whole set, then permits `CleanedUp`. Missing or
    /// stale receipt evidence must fail without releasing any lease.
    async fn finish_cleanup(
        &self,
        receipt: &run_domain::RunCleanupReceipt,
    ) -> Result<Run, RepositoryError>;
}
