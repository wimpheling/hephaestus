//! Forwarding around the actual owned publication provider, without VM wrappers.

#[path = "publication_attempts.rs"]
mod publication_attempts;
#[path = "publication_physical.rs"]
mod publication_physical;

use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec};

use publication_attempts::AttemptLedger;
use publication_physical::{PhysicalObserver, WorkerObservation};

/// Observations of a single actual provision call, without physical cleanup claims.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProvisionOutcome {
    /// The admitted future has not yet returned to the decorator.
    InFlight,
    /// The actual provider returned its unmodified instance handle.
    ReturnedHandle,
    /// The actual provider returned an error; physical cleanup remains to be checked.
    ReturnedError,
    /// The admitted future was dropped; its underlying IO may remain unresolved.
    CancelledOrUnknown,
}

/// A distinct admitted call; duplicate VM identifiers have distinct ledger entries.
#[derive(Clone)]
pub(super) struct ProvisionAttempt {
    id: VmId,
    outcome: ProvisionOutcome,
    physical: Option<WorkerObservation>,
    observation_failed: bool,
}

impl ProvisionAttempt {
    /// Returns the exact identifier copied before the actual provider call.
    pub(super) const fn id(&self) -> &VmId {
        &self.id
    }

    /// Returns the recorded call outcome without interpreting resource absence.
    pub(super) const fn outcome(&self) -> ProvisionOutcome {
        self.outcome
    }
}

/// Passive observation of the original worker after exact scoped cleanup.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum ObservedWorkerExit {
    /// The captured worker PID has no current proc entry.
    Absent { id: VmId, pid: u32 },
    /// The numeric PID now belongs to a different process start identity.
    Reused { id: VmId, pid: u32, start_time: u64 },
}

/// Immutable evidence for tracked calls only, never a physical drain receipt.
pub(super) struct ProvisioningSnapshot {
    closed: bool,
    attempts: Vec<ProvisionAttempt>,
}

impl ProvisioningSnapshot {
    /// Returns all admitted attempts in admission order, including duplicate IDs.
    pub(super) fn attempts(&self) -> &[ProvisionAttempt] {
        &self.attempts
    }

    /// Requires closed admission and actual return of every tracked provision call.
    ///
    /// Returned errors satisfy only call return, not physical cleanup. An empty
    /// closed ledger satisfies only this decorator's call accounting. Neither
    /// case proves worker exit, resource absence, or instance lifecycle completion.
    pub(super) fn require_all_returned(&self) -> Result<(), VmError> {
        if !self.closed {
            return Err(VmError::InvalidState(
                "publication provisioning is still open",
            ));
        }
        if self.attempts.iter().any(|attempt| {
            matches!(
                attempt.outcome,
                ProvisionOutcome::InFlight | ProvisionOutcome::CancelledOrUnknown
            )
        }) {
            return Err(VmError::InvalidState(
                "publication provision calls have unresolved outcomes",
            ));
        }
        Ok(())
    }
}

/// Retains the caller's actual provider and records only decorated provision calls.
///
/// All publication provisioning must use this decorator. Closing it cannot stop
/// calls made directly through other retained clones of the underlying provider.
/// Returned instances remain the actual handles; their lifecycle is not tracked.
pub(super) struct PublicationProvider {
    provider: Arc<dyn VmProvider>,
    scope: VmProviderOwnerScope,
    attempts: AttemptLedger,
    physical: PhysicalObserver,
}

impl PublicationProvider {
    /// Retains an explicitly supplied provider only after its ownership validates.
    ///
    /// No provider, root, ownership namespace, or replacement supervisor is created.
    pub(super) fn new(
        provider: Arc<dyn VmProvider>,
        expected_scope: &VmProviderOwnerScope,
        runtime_root: &Path,
        cgroup_root: &Path,
    ) -> Result<Self, VmError> {
        let scope = provider.owner_scope()?;
        if &scope != expected_scope {
            return Err(VmError::InvalidState("publication physical scope differs"));
        }
        let physical = PhysicalObserver::new(scope.clone(), runtime_root, cgroup_root)?;
        if provider.owner_scope()? != scope {
            return Err(VmError::InvalidState("publication physical scope changed"));
        }
        Ok(Self {
            provider,
            scope,
            attempts: AttemptLedger::new(),
            physical,
        })
    }

    /// Compares the retained actual provider allocation with the caller's `Arc`.
    pub(super) fn retains_provider(&self, provider: &Arc<dyn VmProvider>) -> bool {
        Arc::ptr_eq(&self.provider, provider)
    }

    /// Returns the actual scope validated at construction for later scoped cleanup.
    ///
    /// Cleanup still forwards to the actual provider, which must revalidate scope.
    pub(super) const fn retained_scope(&self) -> &VmProviderOwnerScope {
        &self.scope
    }

    /// Irreversibly closes this decorator's provisioning admission and snapshots it.
    ///
    /// Closure and admission use the same lock. Already admitted calls keep their
    /// records; this does not cancel them, wait for them, or clean up their resources.
    /// Rejected calls do not reach the actual provider and do not enter the ledger.
    pub(super) fn close_provisioning(&self) -> Result<ProvisioningSnapshot, VmError> {
        self.attempts.close()
    }

    /// Passively checks captured workers after the caller performed real scoped cleanup.
    ///
    /// This does not run cleanup, join shutdown, or prove exhaustive native PID capture.
    pub(super) fn verify_physical_after_cleanup(&self) -> Result<Vec<ObservedWorkerExit>, VmError> {
        if self.provider.owner_scope()? != self.scope {
            return Err(VmError::InvalidState("publication physical scope changed"));
        }
        let snapshot = self.attempts.snapshot()?;
        snapshot.require_all_returned()?;
        self.physical.verify(&self.scope, snapshot.attempts())
    }

    /// Snapshots every admitted attempt, including repeated identifiers and errors.
    ///
    /// A closed snapshot can check whether the tracked provision calls returned.
    /// It cannot prove instance destruction, worker exit, or physical absence.
    pub(super) fn provisioning_snapshot(&self) -> Result<ProvisioningSnapshot, VmError> {
        self.attempts.snapshot()
    }
}

#[async_trait]
impl VmProvider for PublicationProvider {
    fn name(&self) -> &'static str {
        self.provider.name()
    }

    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        self.provider.owner_scope()
    }

    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<(), VmError> {
        self.provider.cleanup_orphan_scoped(scope, id).await
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        // An unpolled future has not attempted provider IO. On its first poll,
        // capture this ID before the actual call; cancellation then retains it.
        let id = spec.id.clone();
        let attempt = self.attempts.admit(id.clone())?;
        let result = self.provider.provision(spec).await;
        let physical = result.as_ref().ok().map(|instance| {
            if instance.id() != &id {
                return Err(VmError::InvalidState("publication returned VM ID differs"));
            }
            self.physical.capture(&self.scope, &id)
        });
        // Observation failure holds tracking, but never changes the actual result.
        attempt.returned(result.is_ok(), physical);
        result
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        self.provider.cleanup_orphan(id).await
    }
}
