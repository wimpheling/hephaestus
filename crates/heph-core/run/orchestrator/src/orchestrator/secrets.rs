use super::runtime::{PreparedRunRuntime, RunRuntimeManager};
use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use std::sync::Arc;

/// Prepared ephemeral secret authority for one exact run.
#[derive(Debug, Clone, Default)]
pub struct PreparedRunSecrets {
    /// Read-only secret and runtime-credential mounts added to the VM.
    pub mounts: Vec<vm_trait::VmMount>,
}

/// Lifecycle boundary for exact per-run raw and brokered secret authority.
#[async_trait]
pub trait RunSecretManager: Send + Sync + 'static {
    /// Resolves live authority and materializes only this run's secret files.
    async fn prepare(&self, run: &Run) -> Result<PreparedRunSecrets, RunSecretError>;
    /// Rechecks every prepared lease immediately before VM provisioning.
    async fn reauthorize(&self, run: &Run) -> Result<(), RunSecretError>;
    /// Destroys secret authority after guest destruction is confirmed.
    async fn destroy_after_guest(&self, run_id: RunId) -> Result<(), RunSecretError>;
    /// Reconciles orphan secret files without deleting live guest authority.
    async fn recover(&self) -> Result<usize, RunSecretError>;
}

/// Receives terminal runs only after guest resources and the state lease have
/// been destroyed.
#[async_trait]
pub trait RunCompletionObserver: Send + Sync + 'static {
    /// Applies an idempotent domain decision for one cleaned run.
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError>;
    /// Reconciles cleaned runs whose domain decision was interrupted.
    async fn recover(&self) -> Result<usize, RunCompletionError>;
}

/// Runs several independent post-cleanup observers in a fixed order.
///
/// A run may have more than one durable owner of its terminal outcome. For
/// example, the release updater reconciles update hooks while a mailbox
/// dispatcher records the delivery disposition of normal runs. Keeping those
/// concerns as separate observers avoids teaching the VM orchestrator either
/// domain, while still making recovery execute every durable completion step.
pub struct CompositeRunCompletionObserver {
    observers: Vec<Arc<dyn RunCompletionObserver>>,
}

impl CompositeRunCompletionObserver {
    /// Creates an ordered completion-observer chain.
    #[must_use]
    pub fn new(observers: Vec<Arc<dyn RunCompletionObserver>>) -> Self {
        Self { observers }
    }
}

#[async_trait]
impl RunCompletionObserver for CompositeRunCompletionObserver {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        for observer in &self.observers {
            observer.after_cleanup(run).await?;
        }
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        let mut recovered = 0;
        for observer in &self.observers {
            recovered += observer.recover().await?;
        }
        Ok(recovered)
    }
}

#[derive(Debug)]
pub(super) struct DisabledRunCompletionObserver;

#[async_trait]
impl RunCompletionObserver for DisabledRunCompletionObserver {
    async fn after_cleanup(&self, _run: &Run) -> Result<(), RunCompletionError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}

/// Redacted post-cleanup domain transition failure.
#[derive(Debug, thiserror::Error)]
#[error("run completion observation failed: {message}")]
pub struct RunCompletionError {
    message: String,
}

impl RunCompletionError {
    /// Creates a redacted completion failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct DisabledRunSecretManager;

#[async_trait]
impl RunSecretManager for DisabledRunSecretManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        Ok(PreparedRunSecrets::default())
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunSecretError> {
        Ok(())
    }

    async fn destroy_after_guest(&self, _run_id: RunId) -> Result<(), RunSecretError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunSecretError> {
        Ok(0)
    }
}

/// Non-disclosing exact-secret lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("run secret operation failed: {message}")]
pub struct RunSecretError {
    message: String,
}

impl RunSecretError {
    /// Creates a redacted secret lifecycle failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct DisabledRunRuntimeManager;

#[async_trait]
impl RunRuntimeManager for DisabledRunRuntimeManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        Ok(PreparedRunRuntime::default())
    }

    async fn destroy(&self, _run_id: RunId) -> Result<(), RunRuntimeError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
}

/// Provider-neutral exact-runtime lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("run runtime operation failed: {message}")]
pub struct RunRuntimeError {
    message: String,
}

impl RunRuntimeError {
    /// Creates a redacted runtime lifecycle failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
