//! Atomic owned metadata registration; this is not backing-file ownership proof.

use crate::{VolumeError, VolumeRegistration};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use runtime_types::VolumeId;
use uuid::Uuid;
pub use volume_domain::{
    VolumeCreationIdentity, VolumeCreationIdentityError, VolumeCreationIdentityWire,
    VolumeCreationOperationId, VolumeCreationSealId, VolumeOwnershipScopeId,
};

/// Exact metadata intent plus original effect correlation, without provider handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedVolumeRegistration {
    registration: VolumeRegistration,
    creation: VolumeCreationIdentity,
}
impl OwnedVolumeRegistration {
    /// Combines validated resource intent and validated immutable correlation.
    #[must_use]
    pub const fn new(registration: VolumeRegistration, creation: VolumeCreationIdentity) -> Self {
        Self {
            registration,
            creation,
        }
    }
    /// Exact resource intent.
    #[must_use]
    pub const fn registration(&self) -> &VolumeRegistration {
        &self.registration
    }
    /// Original immutable effect identity; this value does not authorize creation.
    #[must_use]
    pub const fn creation(&self) -> &VolumeCreationIdentity {
        &self.creation
    }
    /// Versioned fixed binary encoding; database SHA-256 fingerprints these bytes.
    ///
    /// UUIDs use their sixteen canonical bytes and counters use big-endian u64.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut result = b"heph-owned-volume-registration-v1\0".to_vec();
        let fields = self.creation.fields();
        for id in [
            self.registration.id().as_uuid(),
            self.registration.project_id(),
            self.registration.filesystem_uuid(),
            fields.scope_id.as_uuid(),
            fields.operation_id.as_uuid(),
            fields.command_id,
            fields.effect_id,
            fields.attempt_id,
            fields.actor_id,
            fields.request_id,
        ] {
            result.extend_from_slice(id.as_bytes());
        }
        for value in [
            self.registration.capacity_bytes(),
            fields.resource_version,
            fields.generation,
        ] {
            result.extend_from_slice(&value.to_be_bytes());
        }
        result.extend_from_slice(&fields.input_hash);
        result.extend_from_slice(&fields.intent_hash);
        result
    }
}

/// Immutable committed metadata lineage, safe to inspect without host handles.
///
/// Only an exact database lookup can establish that this receipt is authentic.
/// This proves neither file creation nor filesystem ownership or absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedVolumeRegistrationReceipt {
    /// Exact original intent and effect correlation.
    pub intent: OwnedVolumeRegistration,
    /// Original metadata creation seal.
    pub seal_id: VolumeCreationSealId,
    /// SHA-256 of the versioned canonical registration bytes.
    pub registration_hash: [u8; 32],
    /// Actual first registration request; replay preserves it.
    pub first_request_id: Uuid,
    /// Exact committed creation event.
    pub event_id: Uuid,
    /// Committed outbox ordering cursor.
    pub event_cursor: i64,
    /// Committed project aggregate version.
    pub event_aggregate_version: i64,
}

/// Authoritative metadata observation; missing metadata never proves file absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeCreationLineage {
    /// No canonical metadata row exists.
    Missing,
    /// Canonical metadata exists without a creation seal; it cannot be adopted.
    Independent,
    /// Exact original metadata creation lineage.
    Owned(Box<OwnedVolumeRegistrationReceipt>),
}

/// Extension of the canonical volume repository, without an alternate provider.
#[async_trait]
pub trait VolumeOwnedRegistrationRepository: Send + Sync + 'static {
    /// Creates metadata and its seal atomically, or returns the original receipt.
    ///
    /// Requires fresh owning-project management before every replay and the same
    /// original actor. An independent or differently sealed row is a conflict.
    async fn register_owned(
        &self,
        identity: &AuthenticatedIdentity,
        intent: &OwnedVolumeRegistration,
    ) -> Result<OwnedVolumeRegistrationReceipt, VolumeError>;
    /// Inspects original lineage after live management authority on this resource.
    /// A current manager may inspect after the creator loses project authority.
    async fn inspect_creation(
        &self,
        identity: &AuthenticatedIdentity,
        volume_id: VolumeId,
    ) -> Result<VolumeCreationLineage, VolumeError>;
    /// Worker-only read of immutable original lineage, with no authority minting.
    ///
    /// It does not accept historical identity as authentication. Workers must
    /// independently match returned lineage to their authoritative original create
    /// claim, including action, identity, hashes, generation, and provenance.
    /// The original claim must precede registration; timestamps alone prove nothing.
    async fn creation_lineage(
        &self,
        volume_id: VolumeId,
    ) -> Result<VolumeCreationLineage, VolumeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_encoding_preserves_every_original_fingerprint() {
        let registration = VolumeRegistration::new(
            VolumeId::new(),
            Uuid::new_v4(),
            16 * 1024 * 1024,
            Uuid::new_v4(),
        )
        .expect("intent");
        let wire = VolumeCreationIdentityWire {
            scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).expect("scope"),
            operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).expect("operation"),
            command_id: Uuid::new_v4(),
            effect_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            resource_version: 2,
            generation: 1,
            input_hash: [1; 32],
            intent_hash: [2; 32],
        };
        let original = OwnedVolumeRegistration::new(
            registration.clone(),
            VolumeCreationIdentity::try_from(wire.clone()).expect("correlation"),
        );
        assert_eq!(original.canonical_bytes(), original.canonical_bytes());
        let mut changed = wire.clone();
        changed.input_hash = [3; 32];
        let changed = OwnedVolumeRegistration::new(
            registration.clone(),
            VolumeCreationIdentity::try_from(changed).expect("changed input"),
        );
        assert_ne!(original.canonical_bytes(), changed.canonical_bytes());
        let mut changed = wire;
        changed.generation += 1;
        let changed = OwnedVolumeRegistration::new(
            registration,
            VolumeCreationIdentity::try_from(changed).expect("different attempt generation"),
        );
        assert_ne!(original.canonical_bytes(), changed.canonical_bytes());
    }
}
