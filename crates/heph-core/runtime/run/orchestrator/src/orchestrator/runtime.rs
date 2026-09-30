use super::{authority::RunAuthorizationError, secrets::RunRuntimeError};
use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;

/// Prepared immutable host-owned files for one exact run.
#[derive(Debug, Clone, Default)]
pub struct PreparedRunRuntime {
    /// Read-only mounts added to the VM specification.
    pub mounts: Vec<vm_trait::VmMount>,
}

/// Lifecycle boundary for exact release artifacts and host-generated context.
#[async_trait]
pub trait RunRuntimeManager: Send + Sync + 'static {
    /// Materializes a fresh, non-reusable runtime tree for one run.
    async fn prepare(&self, run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError>;
    /// Destroys runtime files after provider cleanup has been confirmed.
    async fn destroy(&self, run_id: RunId) -> Result<(), RunRuntimeError>;
    /// Reconciles runtime trees that no live run may reuse.
    async fn recover(&self) -> Result<usize, RunRuntimeError>;
}

/// Live authorization boundary for every logical artifact acquisition and VM
/// start.
#[async_trait]
pub trait RunLaunchAuthorizer: Send + Sync + 'static {
    /// Rechecks the exact run, attachment/update, and release authority.
    async fn authorize(&self, run: &Run) -> Result<(), RunAuthorizationError>;
}

/// Persists exact resources bound to a run before guest provisioning.
///
/// This runs after the fenced volume lease is acquired and the durable run is
/// bound, but before live authorization and VM provisioning. Domain adapters
/// use it to retain delivery-attempt resource evidence.
#[async_trait]
pub trait RunResourceObserver: Send + Sync + 'static {
    /// Records the exact run resource evidence idempotently.
    async fn record(&self, run: &Run) -> Result<(), RunResourceObservationError>;
}

/// Redacted failure while recording pre-provisioning resource evidence.
#[derive(Debug, thiserror::Error)]
#[error("run resource observation failed: {message}")]
pub struct RunResourceObservationError {
    message: String,
}

impl RunResourceObservationError {
    /// Creates a non-disclosing resource-observation failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct DisabledRunResourceObserver;

#[async_trait]
impl RunResourceObserver for DisabledRunResourceObserver {
    async fn record(&self, _run: &Run) -> Result<(), RunResourceObservationError> {
        Ok(())
    }
}
