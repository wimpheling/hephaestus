//! Actor port for explicit typed grants; binding evidence alone is insufficient.

use async_trait::async_trait;
use identity_domain::{AuthenticatedIdentity, UserId};
use volume_domain::{VolumeMountGrantId, VolumeMountRevocationId, VolumeMountScope};

use crate::VolumeError;

/// Audited immutable grant, pinned to one exact consumer revision and mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeMountGrant {
    /// Stable explicit grant identity.
    pub id: VolumeMountGrantId,
    /// Exact frozen binding and declaration scope.
    pub scope: VolumeMountScope,
    /// Creator whose source grant and attachment authority must remain live.
    pub created_by: UserId,
    /// Canonical model revision used during creation.
    pub authorization_model_version: String,
}

/// Immutable permanent withdrawal; a new revision is required to grant again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeMountRevocation {
    /// Predicted revocation record identity.
    pub id: VolumeMountRevocationId,
    /// Exact permanently revoked grant.
    pub grant_id: VolumeMountGrantId,
    /// Authorized actor who withdrew source or consumer authority.
    pub created_by: UserId,
}

/// Live audited actor operations, separate from instance revision activation.
#[async_trait]
pub trait VolumeMountGrantRepository: Send + Sync + 'static {
    /// Creates or replays an exact explicit grant after live two-sided authority.
    /// Revoked grants cannot be replayed or replaced on the same revision.
    async fn grant_volume_mount(
        &self,
        identity: &AuthenticatedIdentity,
        grant_id: VolumeMountGrantId,
        scope: &VolumeMountScope,
    ) -> Result<VolumeMountGrant, VolumeError>;

    /// Permanently withdraws a grant with source-owner or consumer-management authority.
    async fn revoke_volume_mount(
        &self,
        identity: &AuthenticatedIdentity,
        revocation_id: VolumeMountRevocationId,
        grant_id: VolumeMountGrantId,
    ) -> Result<VolumeMountRevocation, VolumeError>;
}
