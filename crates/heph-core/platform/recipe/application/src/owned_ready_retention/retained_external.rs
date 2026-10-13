//! Backend retained-origin comparisons; these values grant no physical custody.

use crate::{
    AttemptProvenance, DeploymentAttemptId, EffectClaim, PreparedOwnedVolumeCreation,
    ResourceProgress,
};
use uuid::Uuid;

/// Qualified original birth and exact new external verification comparison plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedExternalVerificationPlan {
    /// Exact NEW admitted Install/`VerifyExternal` claim, never an old actor identity.
    pub claim: EffectClaim,
    /// Immutable original owned preparation, selected through retained lineage.
    pub preparation: PreparedOwnedVolumeCreation,
    /// Exact permanent original closure.
    pub closure_id: Uuid,
    /// Actual committed positive retention transition operation.
    pub retention_operation_id: DeploymentAttemptId,
    /// Trusted actual opaque VM namespace comparison pin, distinct from disk ownership.
    pub vm_provider_namespace: String,
    /// Trusted actual VM host comparison pin; not provider authority.
    pub vm_host_id: String,
    /// Exactly paired reopening and normal completed/reconciled result; bypasses IO.
    pub completed: Option<ResourceProgress>,
}

/// Selects existing definite completion or a fresh normal reconciliation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedExternalCompletion {
    /// Original claim attempt's genuine Applied worker receipt.
    Completed,
    /// A fresh current-identity verification attempt against the unchanged claim.
    Reconciled(AttemptProvenance),
}
