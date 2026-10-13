//! Backend-only sealed-volume preparation and truthful partial retention.
//!
//! These requests are composition inputs, never public RPC or client receipt tokens.
//! The owning service retains actual non-clone Local custody through recording and
//! atomic actor commit. A DTO or worker receipt never proves an OS lock.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use release_domain::ContentHash;
use runtime_types::VolumeId;
use uuid::Uuid;
use volume_domain::VolumeCreationIdentity;

use crate::{CommandIdentity, DeploymentAttemptId, DeploymentError, EffectClaim, ResourceProgress};

/// Immutable tagged provider preparation committed before any registration or IO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedOwnedVolumeCreation {
    /// Closed mapping version, checked against the exact original claim.
    pub mapping_version: u32,
    /// Exact canonical immutable original claim fingerprint.
    pub claim_digest: ContentHash,
    /// Original provider correlation; labels convey no new authority.
    pub creation: VolumeCreationIdentity,
    /// Exact owned volume predicted by the recipe.
    pub volume_id: VolumeId,
    /// Exact project owning both durable intents.
    pub project_id: Uuid,
    /// Immutable declared geometry, never actual current host length.
    pub capacity_bytes: u64,
    /// Exact original authored filesystem identity.
    pub filesystem_uuid: Uuid,
}

/// Current admitted cleanup command targeting an immutable old owned Create.
///
/// No worker receipt identifier or provider fact is accepted from client Remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainPartialCreation {
    /// Current verified actor's admitted removal command.
    pub command: CommandIdentity,
    /// Exact original immutable owned installation claim.
    pub target: EffectClaim,
    /// Stable backend operation identity, distinct from original provider attempt.
    pub operation_id: DeploymentAttemptId,
    /// Expected deployment progress before this transition.
    pub expected_deployment_version: u64,
    /// Expected resource progress before this transition.
    pub expected_resource_version: u64,
}

/// Authorized backend comparison context, including exact committed replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialCreationRetentionPlan {
    /// Authoritative original preparation; never newly inferred from a matching seal.
    pub preparation: PreparedOwnedVolumeCreation,
    /// Immutable prior result if this exact logical operation already committed.
    pub completed: Option<ResourceProgress>,
}

/// Internal extension implemented by the same canonical recipe repository.
#[async_trait]
pub trait OwnedPartialRetentionRepository: Send + Sync + 'static {
    /// Commits tagged preparation after fresh original Install authorization.
    ///
    /// # Errors
    /// Rejects forged/stale claims, withdrawal, changed replay or non-owned volumes.
    async fn prepare_owned_volume_creation(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
    ) -> Result<PreparedOwnedVolumeCreation, DeploymentError>;

    /// Checks current cleanup authority before discovery or immutable replay.
    ///
    /// # Errors
    /// Rejects borrowed commands, denied management, changed preparation or stale CAS.
    async fn partial_creation_retention_plan(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
    ) -> Result<PartialCreationRetentionPlan, DeploymentError>;

    /// Atomically commits exact receipt/fence, recipe progress, audits and event.
    ///
    /// `worker_receipt_id` is supplied only by the owning backend service while
    /// retaining actual Local custody, never from client Remove. This method does
    /// not manufacture physical custody or a Ready/absence/detach outcome.
    ///
    /// # Errors
    /// Rejects denied replay, stale physical head, foreign lineage, active consumers,
    /// leases, changed inputs or incomplete atomic transition evidence.
    async fn retain_partial_creation(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
        worker_receipt_id: Uuid,
    ) -> Result<ResourceProgress, DeploymentError>;
}
