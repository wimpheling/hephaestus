//! Resource inspection and immutable registration, without provider handles.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use runtime_types::{AgentInstanceId, VolumeId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Volume, VolumeError, VolumeState};

/// Smallest filesystem supported by the local ext4 provider (sixteen mebibytes).
///
/// The historical release declaration's one-byte minimum is a compatibility
/// claim, not a promise that the provider can format a one-byte filesystem.
pub const MIN_LOCAL_VOLUME_CAPACITY_BYTES: u64 = 16 * 1024 * 1024;
/// Largest new registration accepted by this provider (sixteen tebibytes).
pub const MAX_REGISTERED_VOLUME_CAPACITY_BYTES: u64 = 16 * 1024 * 1024 * 1024 * 1024;

/// Immutable resource intent; identifiers are predicted before the first effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeRegistration {
    id: VolumeId,
    project_id: Uuid,
    capacity_bytes: u64,
    filesystem_uuid: Uuid,
}

impl VolumeRegistration {
    /// Validates a new local filesystem intent.
    ///
    /// # Errors
    ///
    /// Rejects nil identifiers, capacities outside the provider bounds, and
    /// capacities that are not aligned to a four-kibibyte filesystem block.
    pub fn new(
        id: VolumeId,
        project_id: Uuid,
        capacity_bytes: u64,
        filesystem_uuid: Uuid,
    ) -> Result<Self, VolumeError> {
        if id.as_uuid().is_nil() || project_id.is_nil() || filesystem_uuid.is_nil() {
            return Err(VolumeError::InvalidState(
                "volume intent identifiers must not be nil",
            ));
        }
        if !(MIN_LOCAL_VOLUME_CAPACITY_BYTES..=MAX_REGISTERED_VOLUME_CAPACITY_BYTES)
            .contains(&capacity_bytes)
            || capacity_bytes % 4096 != 0
        {
            return Err(VolumeError::InvalidState(
                "volume capacity must be aligned and between 16 MiB and 16 TiB",
            ));
        }
        Ok(Self {
            id,
            project_id,
            capacity_bytes,
            filesystem_uuid,
        })
    }

    /// Stable resource identifier.
    #[must_use]
    pub const fn id(&self) -> VolumeId {
        self.id
    }
    /// Owning project identifier.
    #[must_use]
    pub const fn project_id(&self) -> Uuid {
        self.project_id
    }
    /// Exact immutable capacity.
    #[must_use]
    pub const fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }
    /// Expected ext4 filesystem identity.
    #[must_use]
    pub const fn filesystem_uuid(&self) -> Uuid {
        self.filesystem_uuid
    }
}

/// Durable provisioning progress; lease expiry never authorizes formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeProvisioningState {
    /// Intent exists; no successful backing creation has been recorded.
    Reserved,
    /// Creation was authorized; a crash may have left a partial file.
    Creating,
    /// Allocation was synced and formatting may have begun.
    Formatting,
    /// Expected filesystem identity and geometry were proven.
    Ready,
    /// Existing bytes cannot be proven safe; no destructive retry is allowed.
    Uncertain,
}

impl VolumeProvisioningState {
    /// Canonical durable metadata representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Creating => "creating",
            Self::Formatting => "formatting",
            Self::Ready => "ready",
            Self::Uncertain => "uncertain",
        }
    }
}

/// Safe external metadata: no host paths, host identities, or key references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInspection {
    /// Stable resource identity.
    pub id: VolumeId,
    /// Stable owning project.
    pub project_id: Uuid,
    /// Historical origin, if this resource predates standalone registration.
    pub legacy_origin_instance_id: Option<AgentInstanceId>,
    /// Exact persisted capacity.
    pub capacity_bytes: u64,
    /// Persisted filesystem identity, absent only for unreserved legacy volumes.
    pub filesystem_uuid: Option<Uuid>,
    /// Provisioning progress separate from leases.
    pub provisioning_state: VolumeProvisioningState,
    /// Existing attachment lifecycle.
    pub lifecycle_state: VolumeState,
}

/// Trusted provider fence; contains handles and must not cross an actor boundary.
#[derive(Debug, Clone)]
pub struct ProvisioningClaim {
    /// Reserved trusted provider record including its current generation.
    pub volume: Volume,
}

/// Actor-facing resource metadata operations with live audited authorization.
///
/// Registering a resource does not grant an agent attachment authority. Local
/// provisioning is a separate trusted worker operation over the existing row.
#[async_trait]
pub trait VolumeResourceRepository: Send + Sync + 'static {
    /// Registers or replays the exact immutable intent after live owner authority.
    async fn register(
        &self,
        identity: &AuthenticatedIdentity,
        intent: &VolumeRegistration,
    ) -> Result<VolumeInspection, VolumeError>;

    /// Inspects an exact resource after live read authority.
    async fn inspect(
        &self,
        identity: &AuthenticatedIdentity,
        volume_id: VolumeId,
    ) -> Result<VolumeInspection, VolumeError>;

    /// Lists readable retained resources within one authorized project.
    ///
    /// `after` is an exclusive stable identifier cursor; `limit` must be 1..=100.
    async fn list(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        after: Option<VolumeId>,
        limit: u32,
    ) -> Result<Vec<VolumeInspection>, VolumeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_bounds_are_provider_bounds() {
        let id = VolumeId::new();
        let project = Uuid::new_v4();
        let filesystem = Uuid::new_v4();
        for invalid in [
            1,
            MIN_LOCAL_VOLUME_CAPACITY_BYTES - 1,
            MIN_LOCAL_VOLUME_CAPACITY_BYTES + 1,
            MAX_REGISTERED_VOLUME_CAPACITY_BYTES + 4096,
        ] {
            assert!(VolumeRegistration::new(id, project, invalid, filesystem).is_err());
        }
        assert!(
            VolumeRegistration::new(id, project, MIN_LOCAL_VOLUME_CAPACITY_BYTES, filesystem)
                .is_ok()
        );
        assert!(
            VolumeRegistration::new(id, Uuid::nil(), MIN_LOCAL_VOLUME_CAPACITY_BYTES, filesystem)
                .is_err()
        );
    }
}
