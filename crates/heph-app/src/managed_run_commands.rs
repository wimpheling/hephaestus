//! Concrete owning command execution for qualified Invocation and strict Legacy.

mod execute;
mod inputs;
mod intake;
mod lifecycle;
mod ownership;
mod routing;

use crate::{AppError, ManagedInvocationController, ManagedLegacyController};
pub use inputs::{ManagedOrdinaryAdmissionPolicy, ManagedRunCommandInputs};
use run_orchestrator::{OrchestratorError, RepositoryError};
use run_postgres::PgQualifiedInvocationRoutingReader;
use std::sync::Arc;

/// Owns both actual controller families and their immutable original reader.
///
/// Every retained backend, owner, pool and operation-guard Arc is derived from
/// one supplied concrete tuple. No arbitrary controller/provider port is accepted.
/// Separate reader snapshots conservatively select a family; actual controllers
/// requalify persisted profile, producer and owner before effects. Inspection is
/// comparison data, never actor authority, admission or a physical-stop proof.
///
/// Implements [`run_orchestrator::RunCommandExecutor`] without starting workers.
/// Clones share actual controllers, reader and owning-operation intake. Private
/// lifecycle reconciliation performs no global sweep, backend adoption or gate
/// reopening. Constructor success is not listener readiness; the service retains
/// this original tuple through held errors. Drop proves neither stopped nor cleaned.
#[derive(Clone)]
pub struct ManagedRunCommandExecutor {
    invocation: Arc<ManagedInvocationController>,
    legacy: Arc<ManagedLegacyController>,
    reader: Arc<PgQualifiedInvocationRoutingReader>,
    operations: Arc<intake::Operations>,
    ordinary_policy: ManagedOrdinaryAdmissionPolicy,
}

impl ManagedRunCommandExecutor {
    /// Constructs real controllers and the reader from the SAME supplied backend.
    ///
    /// Ordinary inputs clone the one trusted shared tuple. Strict Legacy validates
    /// its additional workspace/account/cgroup/socket relationships and awaits the
    /// actual three-pool preflight before its first initializer. Invocation then
    /// repeats its own checks/preflight before its initializers. Neither constructor
    /// creates/reopens a provider, owner or operation-guard registry.
    ///
    /// # Errors
    /// Propagates actual owner/configuration/root/role/schema or initializer failures.
    /// Construction is not atomic: failed/cancelled assembly may leave directories;
    /// the caller must retain the original backend for subsequent lifecycle work.
    pub async fn new(inputs: ManagedRunCommandInputs) -> Result<Self, AppError> {
        let legacy_inputs = inputs.legacy_inputs();
        let reader = Arc::new(PgQualifiedInvocationRoutingReader::new(
            inputs.shared.pools.worker.clone(),
            inputs.shared.configuration.clone(),
        ));
        let ordinary_policy = inputs.ordinary_policy;
        let legacy = Arc::new(ManagedLegacyController::new(legacy_inputs).await?);
        let invocation = Arc::new(ManagedInvocationController::new(inputs.shared).await?);
        let operations = Arc::new(intake::Operations::new(
            Arc::clone(&invocation),
            Arc::clone(&legacy),
        ));
        Ok(Self {
            invocation,
            legacy,
            reader,
            operations,
            ordinary_policy,
        })
    }
    /// Derives only the actual owning Invocation handle for private App composition.
    pub(super) fn original_invocation(&self) -> Arc<ManagedInvocationController> {
        Arc::clone(&self.invocation)
    }

    /// Seals only this original executor's Start intake; cleanup remains callable.
    pub(super) fn close_intake(&self) -> Result<(), OrchestratorError> {
        self.operations.close()
    }

    /// Owns qualified recovery/stop through waiter cancellation; no public Ready.
    pub(super) async fn reconcile_lifecycle(
        &self,
        cause: run_orchestrator::SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        let owner = self.clone();
        let request = intake::Request::Reconcile(cause);
        let receiver = ownership::spawn(&self.operations, request, async move {
            lifecycle::run(&owner, &owner.operations, cause).await
        })?;
        ownership::receive(receiver).await
    }
}

fn held(message: &'static str) -> OrchestratorError {
    RepositoryError::InvalidData(message).into()
}
