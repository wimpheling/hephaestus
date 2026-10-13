//! Exact VM observations are data until a trusted worker persists their proof.

use super::{RunCleanupError, RunCleanupHostId, RunCleanupTarget, validate_provider_identity};
use time::OffsetDateTime;
use vm_trait::VmId;

/// Provenance of a VM identity that a cleanup worker must reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunCleanupVmTargetKind {
    /// Identity stored with the run's provisioned resources.
    Persisted,
    /// Exact identity durably planned before any provisioning IO.
    Planned,
    /// Historical custom identity cannot yet be resolved authoritatively.
    HistoricalUnresolved,
}

/// A checked provider host scope and VM identity, or unresolved history.
///
/// A planned target is still checked through the provider for destruction or
/// authoritative absence. It never means that provisioning did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupVmTarget {
    kind: RunCleanupVmTargetKind,
    host: Option<RunCleanupHostId>,
    vm_id: Option<VmId>,
}

impl RunCleanupVmTarget {
    /// Constructs the exact VM identity recovered from durable binding evidence.
    ///
    /// # Errors
    ///
    /// Rejects empty, excessive, or unsafe provider identifiers.
    pub fn persisted(host: RunCleanupHostId, vm_id: VmId) -> Result<Self, RunCleanupError> {
        validate_provider_identity(&vm_id.0)?;
        Ok(Self {
            kind: RunCleanupVmTargetKind::Persisted,
            host: Some(host),
            vm_id: Some(vm_id),
        })
    }

    /// Constructs the exact target persisted before provisioning IO begins.
    ///
    /// # Errors
    ///
    /// Rejects empty, excessive, or unsafe provider identifiers.
    pub fn planned(host: RunCleanupHostId, vm_id: VmId) -> Result<Self, RunCleanupError> {
        validate_provider_identity(&vm_id.0)?;
        Ok(Self {
            kind: RunCleanupVmTargetKind::Planned,
            host: Some(host),
            vm_id: Some(vm_id),
        })
    }

    /// Retains explicit uncertainty, preventing any cleanup receipt.
    ///
    /// Never fabricate a historical custom VM identity from its run UUID. A
    /// legacy canonical mapping needs actual durable factory/provider evidence
    /// for both VM and provider host; never use the current process's host.
    #[must_use]
    pub const fn historical_unresolved() -> Self {
        Self {
            kind: RunCleanupVmTargetKind::HistoricalUnresolved,
            host: None,
            vm_id: None,
        }
    }

    /// Returns the identity provenance that is included in the target digest.
    #[must_use]
    pub const fn kind(&self) -> RunCleanupVmTargetKind {
        self.kind
    }

    /// Returns the exact VM identity; absence represents unresolved history.
    #[must_use]
    pub const fn vm_id(&self) -> Option<&VmId> {
        self.vm_id.as_ref()
    }

    /// Returns the exact host namespace, or unresolved historical scope.
    #[must_use]
    pub const fn host(&self) -> Option<&RunCleanupHostId> {
        self.host.as_ref()
    }
}

/// Physical observation a trusted provider boundary must establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunCleanupVmObservationKind {
    /// Destruction of the exact VM and its processes was confirmed.
    Destroyed,
    /// The provider authoritatively confirmed that the exact VM is absent.
    AuthoritativelyAbsent,
}

/// Description of a provider observation, without authority to release a lease.
///
/// Returning success from an arbitrary caller or seeing no in-memory handle
/// does not establish either observation. Later worker persistence must verify
/// the provider result for the exact durable target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupVmObservation {
    kind: RunCleanupVmObservationKind,
    host: RunCleanupHostId,
    vm_id: VmId,
}

impl RunCleanupVmObservation {
    /// Describes provider-confirmed destruction of an exact identity.
    ///
    /// # Errors
    ///
    /// Rejects invalid provider identity shape; this does not verify destruction.
    pub fn destroyed(host: RunCleanupHostId, vm_id: VmId) -> Result<Self, RunCleanupError> {
        validate_provider_identity(&vm_id.0)?;
        Ok(Self {
            kind: RunCleanupVmObservationKind::Destroyed,
            host,
            vm_id,
        })
    }

    /// Describes provider-authoritative absence of an exact identity.
    ///
    /// # Errors
    ///
    /// Rejects invalid identity shape; this does not verify provider absence.
    pub fn authoritatively_absent(
        host: RunCleanupHostId,
        vm_id: VmId,
    ) -> Result<Self, RunCleanupError> {
        validate_provider_identity(&vm_id.0)?;
        Ok(Self {
            kind: RunCleanupVmObservationKind::AuthoritativelyAbsent,
            host,
            vm_id,
        })
    }

    /// Returns the claimed observation for later trusted verification.
    #[must_use]
    pub const fn kind(&self) -> RunCleanupVmObservationKind {
        self.kind
    }

    /// Returns the exact observed VM identity.
    #[must_use]
    pub const fn vm_id(&self) -> &VmId {
        &self.vm_id
    }

    /// Returns the exact provider namespace and host making the observation.
    #[must_use]
    pub const fn host(&self) -> &RunCleanupHostId {
        &self.host
    }
}

/// Exact cleanup result description shared by all failure and recovery paths.
///
/// This public value is not proof. Only a trusted worker may persist verified
/// provider observations and atomically consume their matching target. Actors
/// cannot establish cleanup, release fences, or settle a mailbox through this DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupReceipt {
    target: RunCleanupTarget,
    observation: RunCleanupVmObservation,
    observed_at: OffsetDateTime,
}

impl RunCleanupReceipt {
    /// Validates an observation against the full immutable target.
    ///
    /// Observation time is truncated to microsecond precision so durable
    /// `PostgreSQL` receipt round trips preserve exact equality. Time is not
    /// an authority or lease fence.
    ///
    /// # Errors
    ///
    /// Rejects unresolved historical scope, a different provider namespace or
    /// host, or a different observed VM.
    pub fn new(
        target: RunCleanupTarget,
        observation: RunCleanupVmObservation,
        observed_at: OffsetDateTime,
    ) -> Result<Self, RunCleanupError> {
        let expected = target
            .vm_target()
            .vm_id()
            .ok_or(RunCleanupError::UnresolvedVmIdentity)?;
        if target.vm_target().host() != Some(observation.host()) {
            return Err(RunCleanupError::HostMismatch);
        }
        if expected != observation.vm_id() {
            return Err(RunCleanupError::VmMismatch);
        }
        let observed_at =
            observed_at - time::Duration::nanoseconds(i64::from(observed_at.nanosecond() % 1_000));
        Ok(Self {
            target,
            observation,
            observed_at,
        })
    }

    /// Returns the complete target that must still match persisted closed leases.
    #[must_use]
    pub const fn target(&self) -> &RunCleanupTarget {
        &self.target
    }

    /// Returns the observation requiring trusted provider verification.
    #[must_use]
    pub const fn observation(&self) -> &RunCleanupVmObservation {
        &self.observation
    }

    /// Returns the worker's observation time, without treating it as a fence.
    #[must_use]
    pub const fn observed_at(&self) -> OffsetDateTime {
        self.observed_at
    }

    /// Compares all exact target fields, including generation and complete set.
    ///
    /// # Errors
    ///
    /// Rejects changed run/revision/VM/lease/host/fence identity or generation.
    pub fn matches_target(&self, target: &RunCleanupTarget) -> Result<(), RunCleanupError> {
        if &self.target != target {
            return Err(RunCleanupError::TargetMismatch);
        }
        Ok(())
    }
}
