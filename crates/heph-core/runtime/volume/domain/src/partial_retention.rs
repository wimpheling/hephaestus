//! Positive partial birth facts; these checked values never establish a host lock.

use crate::{
    MAX_VOLUME_CAPACITY_BYTES, VolumeCreationIdentity, VolumeCreationIdentityWire,
    VolumeCreationSealId, VolumeRootNamespaceId,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A positive owned birth phase which conveys neither Ready nor resource absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedPartialBirthPhase {
    /// Complete namespace/inode birth journal identifies an empty claimed inode.
    ClaimedEmpty,
    /// Exact capacity exists with complete positive never-format journal evidence.
    AllocatedNeverFormat,
    /// Format intent exists but initial clean filesystem readiness is unproven.
    FormatIntentIncomplete,
}
impl OwnedPartialBirthPhase {
    /// Returns the closed storage vocabulary.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaimedEmpty => "claimed_empty",
            Self::AllocatedNeverFormat => "allocated_never_format",
            Self::FormatIntentIncomplete => "format_intent_incomplete",
        }
    }
}

/// An identifier to look up an immutable worker receipt, never a reusable grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct OwnedPartialRetentionReceiptId(Uuid);
impl OwnedPartialRetentionReceiptId {
    /// Checks a nonnil durable receipt label without authenticating any receipt.
    ///
    /// # Errors
    /// Rejects nil identifiers.
    pub const fn from_uuid(id: Uuid) -> Result<Self, PartialRetentionContractError> {
        if id.is_nil() {
            Err(PartialRetentionContractError)
        } else {
            Ok(Self(id))
        }
    }
    /// Returns the durable lookup identifier.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}
impl TryFrom<Uuid> for OwnedPartialRetentionReceiptId {
    type Error = PartialRetentionContractError;
    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        Self::from_uuid(value)
    }
}
impl From<OwnedPartialRetentionReceiptId> for Uuid {
    fn from(value: OwnedPartialRetentionReceiptId) -> Self {
        value.0
    }
}

/// Pure validation failure; unknown host birth must remain held instead of guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("partial birth facts differ from exact positive owned evidence")]
pub struct PartialRetentionContractError;

/// Untrusted observation fields; a provider must independently establish each fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedPartialBirthObservationWire {
    /// Unique immutable observation identity.
    pub id: Uuid,
    /// Monotonic physical-observation head version, not the creation generation.
    pub observation_version: u64,
    /// Exact canonical resource and project; predicted equality proves no ownership.
    pub volume_id: Uuid,
    /// Exact canonical volume project.
    pub project_id: Uuid,
    /// Original immutable sealed metadata birth.
    pub seal_id: VolumeCreationSealId,
    /// Complete original effect identity, independently matched by the trusted engine.
    pub original_creation: VolumeCreationIdentityWire,
    /// Exact original admitted provisioning operation, independent of birth generation.
    pub provisioning_operation_id: Uuid,
    /// Exact immutable purpose digest, including original seal and root owner UUID.
    pub purpose_hash: [u8; 32],
    /// Exact current provider provisioning fence.
    pub generation: u64,
    /// Positive partial phase; no unknown or Ready variant exists.
    pub phase: OwnedPartialBirthPhase,
    /// Immutable intended capacity, distinct from observed file length.
    pub declared_capacity_bytes: u64,
    /// Actual descriptor length; zero is allowed only for a positively claimed empty inode.
    pub actual_length_bytes: u64,
    /// Persistent physical provider root owner label, independently verified on host.
    pub owner_namespace: VolumeRootNamespaceId,
    /// Exact provider-root device.
    pub root_device: u64,
    /// Exact provider-root inode.
    pub root_inode: u64,
    /// Exact independently checked provider-root owner UID.
    pub root_owner_uid: u32,
    /// Exact private journal namespace device.
    pub namespace_device: u64,
    /// Exact private journal namespace inode.
    pub namespace_inode: u64,
    /// Actual private namespace and backing owner UID.
    pub owner_uid: u32,
    /// Exact recorded backing device.
    pub backing_device: u64,
    /// Exact recorded backing inode.
    pub backing_inode: u64,
    /// Complete durable namespace birth fingerprint.
    pub birth_record_hash: [u8; 32],
    /// Complete original backing inode record fingerprint.
    pub inode_record_hash: [u8; 32],
    /// Exact contradiction-free journal phase evidence fingerprint.
    pub journal_hash: [u8; 32],
}

/// Checked observation data, serializable only; this is not physical/provider proof.
///
/// Parsing cannot establish inode ownership, quiescence or an OS flock. Trusted
/// composition must retain the provider's opaque physical witness through worker
/// recording and the actor's atomic permanent-fence/ledger transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OwnedPartialBirthObservation(OwnedPartialBirthObservationWire);
impl OwnedPartialBirthObservation {
    /// Returns exact bounded comparison data without conferring authority.
    #[must_use]
    pub const fn fields(&self) -> &OwnedPartialBirthObservationWire {
        &self.0
    }

    /// Compares every original effect field without treating equality as authority.
    ///
    /// The service must load the authoritative preceding Create claim, compare
    /// its canonical provider-operation mapping and independently hold custody.
    #[must_use]
    pub fn matches_creation(&self, original: &VolumeCreationIdentity) -> bool {
        &self.0.original_creation == original.fields()
    }
}
impl TryFrom<OwnedPartialBirthObservationWire> for OwnedPartialBirthObservation {
    type Error = PartialRetentionContractError;
    fn try_from(value: OwnedPartialBirthObservationWire) -> Result<Self, Self::Error> {
        let positive = [
            value.observation_version,
            value.generation,
            value.root_inode,
            value.namespace_inode,
            value.backing_inode,
        ];
        let devices = [
            value.root_device,
            value.namespace_device,
            value.backing_device,
        ];
        let length_valid = match value.phase {
            OwnedPartialBirthPhase::ClaimedEmpty => value.actual_length_bytes == 0,
            OwnedPartialBirthPhase::AllocatedNeverFormat
            | OwnedPartialBirthPhase::FormatIntentIncomplete => {
                value.actual_length_bytes == value.declared_capacity_bytes
            }
        };
        if [
            value.id,
            value.volume_id,
            value.project_id,
            value.provisioning_operation_id,
        ]
        .iter()
        .any(Uuid::is_nil)
            || VolumeCreationIdentity::try_from(value.original_creation.clone()).is_err()
            || positive.contains(&0)
            || positive
                .iter()
                .chain(&devices)
                .any(|value| *value > i64::MAX.cast_unsigned())
            || value.declared_capacity_bytes == 0
            || value.declared_capacity_bytes > MAX_VOLUME_CAPACITY_BYTES
            || value.declared_capacity_bytes % 4096 != 0
            || !length_valid
        {
            return Err(PartialRetentionContractError);
        }
        Ok(Self(value))
    }
}

#[cfg(test)]
#[path = "partial_retention_tests.rs"]
mod tests;
