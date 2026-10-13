use serde::{Deserialize, Serialize};
use uuid::Uuid;
use volume_domain::OwnedBackingPhase;

use crate::VolumeError;

/// Readonly initial-filesystem birth observation, distinct from later runtime reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedFilesystemBirth {
    /// Exact observed filesystem UUID.
    pub filesystem_uuid: Uuid,
    /// Readonly observed block size.
    pub block_size: u32,
    /// Readonly observed block count.
    pub block_count: u64,
    /// Initial filesystem clean state; Ready rejects dirty birth observations.
    pub clean: bool,
}

/// Untrusted wire facts; only the canonical worker boundary may record them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedBackingObservationWire {
    /// Stable observation identity; changed facts under the same ID conflict.
    pub id: Uuid,
    /// Exact immutable purpose fingerprint.
    pub purpose_hash: [u8; 32],
    /// Current CAS fence, distinct from immutable purpose birth generation.
    pub generation: u64,
    /// Closed physical birth phase.
    pub phase: OwnedBackingPhase,
    /// Verified provider root device.
    pub root_device: u64,
    /// Verified provider root inode.
    pub root_inode: u64,
    /// Verified private journal namespace device.
    pub namespace_device: u64,
    /// Verified private journal namespace inode.
    pub namespace_inode: u64,
    /// Actual private namespace and backing owner UID.
    pub owner_uid: u32,
    /// Exact recorded backing device.
    pub backing_device: u64,
    /// Exact recorded backing inode.
    pub backing_inode: u64,
    /// SHA-256 of complete durable birth record.
    pub birth_record_hash: [u8; 32],
    /// SHA-256 of exact durable inode record.
    pub inode_record_hash: [u8; 32],
    /// SHA-256 of the observed complete phase journal evidence.
    pub journal_hash: [u8; 32],
    /// Positive contradiction-free `NeverFormat` proof, required before retry.
    pub never_format_started: bool,
    /// Exact allocated/published file size observation.
    pub capacity_bytes: u64,
    /// Initial clean filesystem observation only for Ready.
    pub filesystem: Option<OwnedFilesystemBirth>,
}

/// Checked metadata DTO; constructing it does not establish physical proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OwnedBackingObservation(OwnedBackingObservationWire);
impl OwnedBackingObservation {
    /// Returns checked observations, which still require trusted worker provenance.
    #[must_use]
    pub const fn fields(&self) -> &OwnedBackingObservationWire {
        &self.0
    }
}
impl TryFrom<OwnedBackingObservationWire> for OwnedBackingObservation {
    type Error = VolumeError;
    fn try_from(value: OwnedBackingObservationWire) -> Result<Self, Self::Error> {
        let counters = [
            value.generation,
            value.root_inode,
            value.namespace_inode,
            value.backing_inode,
            value.capacity_bytes,
        ];
        if value.id.is_nil()
            || counters.contains(&0)
            || counters
                .iter()
                .any(|counter| *counter > i64::MAX.cast_unsigned())
            || [
                value.root_device,
                value.namespace_device,
                value.backing_device,
            ]
            .iter()
            .any(|device| *device > i64::MAX.cast_unsigned())
            || (matches!(
                value.phase,
                OwnedBackingPhase::Recorded | OwnedBackingPhase::FailedBeforeFormat
            ) != value.never_format_started)
        {
            return Err(VolumeError::IntentConflict);
        }
        match (&value.filesystem, value.phase) {
            (Some(filesystem), OwnedBackingPhase::Ready)
                if filesystem.clean
                    && !filesystem.filesystem_uuid.is_nil()
                    && filesystem.block_size == 4096
                    && filesystem
                        .block_count
                        .checked_mul(u64::from(filesystem.block_size))
                        == Some(value.capacity_bytes) => {}
            (None, phase) if phase != OwnedBackingPhase::Ready => {}
            _ => return Err(VolumeError::IntentConflict),
        }
        Ok(Self(value))
    }
}
