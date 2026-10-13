//! Read-only original Invocation comparison; no admission or controller selection.

mod ordinary;
mod projection;
pub mod snapshot;

pub use ordinary::{
    QualifiedOrdinaryEvidence, QualifiedOrdinaryGitSource, QualifiedOrdinaryRoute,
    QualifiedOrdinaryRoutingDecision, QualifiedOrdinaryRoutingHoldReason, QualifiedOrdinaryUnborn,
};
pub use projection::{
    QualifiedInvocationEvidence, QualifiedInvocationRoute, QualifiedInvocationRoutingDecision,
    QualifiedInvocationRoutingHoldReason,
};

use release_service::InstanceExecutionConfiguration;
use run_domain::StartRun;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::PgPool;

use crate::errors::storage;

/// Worker-only reader bound to one trusted, checked execution configuration.
///
/// Neither construction nor a qualified projection proves physical ownership,
/// grants execution rights, or replaces controller requalification before IO.
/// Invocation inspection supports the exact protected Invocation originals.
/// Separate ordinary inspection supports strict Legacy Git Normal originals;
/// its unborn projection selects no profile and admits no execution.
pub struct PgQualifiedInvocationRoutingReader {
    worker_pool: PgPool,
    expected: InstanceExecutionConfiguration,
}

impl PgQualifiedInvocationRoutingReader {
    /// Retains the actual supplied worker pool and checked expected configuration.
    ///
    /// The owning composition must validate actual provider/root ownership first.
    /// Every inspection independently checks its acquired connection's role.
    #[must_use]
    pub const fn new(worker_pool: PgPool, expected: InstanceExecutionConfiguration) -> Self {
        Self {
            worker_pool,
            expected,
        }
    }

    /// Compares one delivery with the exact persisted original Invocation Start.
    ///
    /// The payload's kind never selects a profile. Cancelled/closed originals
    /// may qualify for comparison while the controller still denies Start IO.
    ///
    /// # Errors
    /// Returns storage/transaction failures; unknown or conflicting originals
    /// return a typed held decision. Every ordinary return awaits rollback.
    pub async fn inspect_start(
        &self,
        command: &StartRun,
    ) -> Result<QualifiedInvocationRoutingDecision, RepositoryError> {
        self.inspect(command.run_id, Some(command)).await
    }

    /// Identifies an immutable original after cancellation, closure or completion.
    ///
    /// Original actor/request IDs are comparison data, never authenticated
    /// principals. No current actor rights or dispatch eligibility are queried.
    /// This is not Cancel/Wake authority or a physical absence/stopped proof.
    ///
    /// # Errors
    /// Returns storage/transaction failures; unknown or conflicting originals
    /// return a typed held decision. Every ordinary return awaits rollback.
    pub async fn inspect_original(
        &self,
        run: RunId,
    ) -> Result<QualifiedInvocationRoutingDecision, RepositoryError> {
        self.inspect(run, None).await
    }

    /// Compares a delivery with a strict Legacy Git Normal original or unborn request.
    ///
    /// The globally persisted Run is inspected before profile/owner selection.
    /// Canonical and other ordinary producers remain explicitly held until their
    /// immutable worker contracts are supported. Invocation inspection is separate.
    ///
    /// # Errors
    /// Propagates setup/storage/transaction failures. Every ordinary return awaits
    /// rollback; unsupported, partial or conflicting originals return typed Held.
    pub async fn inspect_ordinary_start(
        &self,
        command: &StartRun,
    ) -> Result<QualifiedOrdinaryRoutingDecision, RepositoryError> {
        self.inspect_ordinary(command.run_id, Some(command)).await
    }

    /// Reads an ordinary immutable original for original-Run Cancel comparison.
    ///
    /// Cancellation, closure, completion and current dispatch disposition do not
    /// select another family. A prospective unborn result has no durable profile.
    /// Neither result grants Cancel/Wake authority, admission or physical IO.
    ///
    /// # Errors
    /// Propagates setup/storage/transaction failures and awaits ordinary rollbacks.
    pub async fn inspect_ordinary_original(
        &self,
        run: RunId,
    ) -> Result<QualifiedOrdinaryRoutingDecision, RepositoryError> {
        self.inspect_ordinary(run, None).await
    }

    async fn inspect_ordinary(
        &self,
        run: RunId,
        command: Option<&StartRun>,
    ) -> Result<QualifiedOrdinaryRoutingDecision, RepositoryError> {
        let mut transaction = self.worker_pool.begin().await.map_err(storage)?;
        let result = ordinary::inspect(&mut transaction, &self.expected, run, command).await;
        let rollback = transaction.rollback().await.map_err(storage);
        match result {
            Ok(decision) => rollback.map(|()| decision),
            Err(primary) => {
                drop(rollback);
                Err(primary)
            }
        }
    }

    async fn inspect(
        &self,
        run: RunId,
        command: Option<&StartRun>,
    ) -> Result<QualifiedInvocationRoutingDecision, RepositoryError> {
        let mut transaction = self.worker_pool.begin().await.map_err(storage)?;
        let result = snapshot::inspect(&mut transaction, &self.expected, run, command).await;
        // All expected denials and setup/query failures return through here.
        // Cancellation cannot promise an awaited rollback from Transaction Drop.
        let rollback = transaction.rollback().await.map_err(storage);
        match result {
            Ok(decision) => rollback.map(|()| decision),
            Err(primary) => {
                drop(rollback);
                Err(primary)
            }
        }
    }
}
