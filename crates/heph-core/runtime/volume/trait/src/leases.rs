use runtime_types::{LeaseId, RunId, VolumeId};
use time::OffsetDateTime;

use crate::Volume;

/// Exclusive writable claim held by one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLease {
    /// Stable lease identifier.
    pub id: LeaseId,
    /// Leased volume.
    pub volume_id: VolumeId,
    /// Run holding the lease.
    pub run_id: RunId,
    /// Host on which the volume may be attached.
    pub host_id: String,
    /// Monotonic fencing generation for the volume.
    pub fencing_token: i64,
    /// Time at which the lease was acquired.
    pub acquired_at: OffsetDateTime,
    /// Most recent supervisor heartbeat.
    pub heartbeat_at: OffsetDateTime,
    /// Time after which the lease is eligible for supervised recovery.
    pub expires_at: OffsetDateTime,
    /// Time at which VM attachment was confirmed.
    pub attached_at: Option<OffsetDateTime>,
}

/// Information needed to attach a leased volume to a VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeAttachment {
    /// Persistent volume metadata.
    pub volume: Volume,
    /// Lease authorizing this writable attachment.
    pub lease: VolumeLease,
    /// Stable `VmDisk` identifier.
    pub disk_id: &'static str,
}
