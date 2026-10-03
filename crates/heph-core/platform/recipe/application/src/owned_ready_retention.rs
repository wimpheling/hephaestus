//! Current-manager retention of an actual sealed Ready birth.
//!
//! Original installation history remains unchanged. These backend comparison
//! values neither grant provider ownership nor stand in for actual Local custody.

use crate::{
    CommandIdentity, DeploymentAttemptId, DeploymentError, EffectClaim,
    PreparedOwnedVolumeCreation, ResourceProgress,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

/// An admitted current Remove targeting one immutable original owned Create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainOwnedReady {
    /// Current manager's admitted removal command.
    pub command: CommandIdentity,
    /// Exact original installation claim; never a reconstructed actor identity.
    pub target: EffectClaim,
    /// Stable distinct retention operation, reused on retry.
    pub operation_id: DeploymentAttemptId,
    /// Expected deployment progress before the distinct Remove transition.
    pub expected_deployment_version: u64,
    /// Expected resource progress before that transition.
    pub expected_resource_version: u64,
}

/// Checked original preparation and permanent admission closure comparison data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedReadyRetentionPlan {
    /// Original protected preparation; its receipt cannot be rewritten by Remove.
    pub preparation: PreparedOwnedVolumeCreation,
    /// Exact immutable durable closure, supplied only by the backend adapter.
    pub closure_id: Uuid,
    /// Trusted actual opaque VM namespace comparison pin, distinct from disk ownership.
    pub vm_provider_namespace: String,
    /// Trusted actual VM host comparison pin; grants no provider authority.
    pub vm_host_id: String,
    /// Positively committed exact birth result, which bypasses physical work.
    pub completed: Option<ResourceProgress>,
}

/// Fresh external verification requesting explicit reuse of a retained volume.
///
/// This never revives the removed consumer or grants runtime mounting authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReopenRetainedOwnedVolume {
    /// Active exact external verification claim from the new admitted Install.
    pub claim: EffectClaim,
    /// Exact positive retained disposition, discovered by backend composition.
    pub retention_operation_id: DeploymentAttemptId,
}

/// A new current-manager command or a read-only already retained result.
#[derive(Debug, Clone)]
pub enum OwnedReadyRemoveResume {
    /// Fresh real Remove admission at the existing closed Removing CAS.
    Admitted(Box<crate::DeploymentAdmission>),
    /// Qualified terminal disposition; no command or effect is created.
    Retained(ResourceProgress),
}

/// Distinct actor boundary for positive Ready retention and explicit reuse.
#[async_trait]
pub trait OwnedReadyRetentionRepository: Send + Sync + 'static {
    /// Admits a fresh current-manager Remove on the same positively closed birth.
    ///
    /// Rechecks cleanup permissions for ALL owned resources. The new immutable
    /// command uses current Removing CAS without rewriting closure or deployment
    /// state/version. A fully retained target returns read-only progress instead.
    /// This distinct admission never uses generic receipt-version minus one.
    ///
    /// # Errors
    /// Unsupported, drift, borrowed commands, denied management or stale CAS deny.
    async fn resume_owned_ready_remove(
        &self,
        _identity: &AuthenticatedIdentity,
        _remove: &crate::RemoveDeployment,
        _target: &EffectClaim,
    ) -> Result<OwnedReadyRemoveResume, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }

    /// Commits permanent admission closure before any drain or physical probing.
    ///
    /// Requires a current active project/volume manager and exact original birth.
    /// Source permission is not borrowed from the original installation actor.
    ///
    /// # Errors
    /// Unsupported implementations, drift, stale CAS or unproven lineage deny.
    async fn begin_owned_ready_retention(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &RetainOwnedReady,
    ) -> Result<OwnedReadyRetentionPlan, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
    /// Commits the exact positive worker fact with the distinct Remove transition.
    ///
    /// Composition must retain actual non-clone Local custody until this awaited
    /// transaction resolves. A fact identifier is never accepted from public Remove.
    ///
    /// # Errors
    /// Denies lost authority, active consumers/leases, stale facts or changed CAS.
    async fn retain_owned_ready(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &RetainOwnedReady,
        _worker_fact_id: Uuid,
    ) -> Result<ResourceProgress, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
    /// Reopens only for a fresh authorized exact external verification claim.
    ///
    /// Requires current whole preparation and SourceUse for the NEW consumer,
    /// exact retained origin and immutable same-project volume/access/capacity.
    /// Old Remove neither requires SourceUse nor fabricates Install completion.
    ///
    /// # Errors
    /// Unsupported, historical, incomplete or foreign retained origins deny.
    async fn reopen_retained_owned_volume(
        &self,
        _identity: &AuthenticatedIdentity,
        _request: &ReopenRetainedOwnedVolume,
    ) -> Result<(), DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }
}
