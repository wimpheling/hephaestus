//! Provider-neutral contracts for persistent agent volumes.

use async_trait::async_trait;
use runtime_types::{AgentId, LeaseId, RunId, VolumeId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::OffsetDateTime;
use uuid::Uuid;

/// Stable block-device identifier used for the agent state disk.
pub const AGENT_STATE_DISK_ID: &str = "agent-state";

/// The purpose of a persistent volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum VolumeKind {
    /// Per-agent `SQLite` state.
    AgentState,
}

/// Durable lifecycle state of a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum VolumeState {
    /// The backing file has been allocated but is not ready for attachment.
    Uninitialized,
    /// The filesystem is formatted and ready for use.
    ///
    /// A separate active lease row may reserve a ready volume while VM
    /// attachment is in progress.
    Ready,
    /// A leased volume is attached to a VM.
    Attached,
    /// An abandoned lease is being fenced and cleaned up.
    Recovering,
}

/// Durable metadata describing one persistent volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// Stable volume identifier.
    pub id: VolumeId,
    /// Agent that owns the volume.
    pub agent_id: AgentId,
    /// Purpose of the volume.
    pub kind: VolumeKind,
    /// Host that owns the local backing file.
    pub host_id: String,
    /// Caller-owned raw backing file.
    pub host_path: PathBuf,
    /// Capacity in bytes.
    pub capacity_bytes: u64,
    /// Stable `ext4` filesystem `UUID`.
    pub filesystem_uuid: Uuid,
    /// Current durable state.
    pub state: VolumeState,
    /// External encryption-key reference, when encryption is introduced.
    pub key_reference: Option<String>,
    /// Encryption metadata format version.
    pub encryption_version: Option<i32>,
    /// Latest durable backup revision.
    pub backup_revision: Option<i64>,
    /// Checksum associated with the latest backup.
    pub checksum: Option<String>,
    /// Completion time of the latest successful backup.
    pub last_successful_backup_at: Option<OffsetDateTime>,
}

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

/// Failure returned by a volume store.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VolumeError {
    /// The requested volume does not exist.
    #[error("volume {0} was not found")]
    NotFound(VolumeId),
    /// Another run owns the exclusive writable lease.
    #[error("volume {volume_id} is leased by run {holder_run_id}")]
    LeaseConflict {
        /// Conflicting volume.
        volume_id: VolumeId,
        /// Current lease holder.
        holder_run_id: RunId,
    },
    /// A lease token does not match the current active lease.
    #[error("volume lease is no longer current")]
    StaleLease,
    /// A volume belongs to another host.
    #[error("volume belongs to host {owner_host}, not {requested_host}")]
    WrongHost {
        /// Durable owner host.
        owner_host: String,
        /// Host requesting attachment.
        requested_host: String,
    },
    /// A state transition violates the volume lifecycle.
    #[error("invalid volume state transition: {0}")]
    InvalidState(&'static str),
    /// Persistent metadata access failed.
    #[error("volume metadata operation failed: {0}")]
    Metadata(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Local backing storage access failed.
    #[error("volume backing operation failed: {0}")]
    Backing(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistent volume operations used by the run orchestrator.
#[async_trait]
pub trait VolumeStore: Send + Sync + 'static {
    /// Creates or resolves the single agent-state volume for `agent_id`.
    ///
    /// The returned backing file is formatted by the host before the volume
    /// enters [`VolumeState::Ready`].
    ///
    /// # Errors
    ///
    /// Returns an error when metadata or backing-file creation fails.
    async fn resolve_agent_state(
        &self,
        agent_id: AgentId,
        capacity_bytes: u64,
    ) -> Result<Volume, VolumeError>;

    /// Acquires the exclusive writable lease for a run.
    ///
    /// # Errors
    ///
    /// Returns [`VolumeError::LeaseConflict`] while another run holds the
    /// active lease.
    async fn acquire(
        &self,
        volume_id: VolumeId,
        run_id: RunId,
    ) -> Result<VolumeAttachment, VolumeError>;

    /// Records that the VM successfully attached the leased disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the supplied lease is no longer current.
    async fn mark_attached(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError>;

    /// Extends a live lease heartbeat.
    ///
    /// # Errors
    ///
    /// Returns an error if the supplied lease is no longer current.
    async fn heartbeat(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError>;

    /// Returns the active lease held by `run_id`, when one exists.
    ///
    /// # Errors
    ///
    /// Returns an error when durable metadata cannot be read.
    async fn active_lease_for_run(&self, run_id: RunId)
    -> Result<Option<VolumeLease>, VolumeError>;

    /// Releases a lease after VM destruction confirmed disk detachment.
    ///
    /// # Errors
    ///
    /// Returns an error if the supplied lease is no longer current.
    async fn release_after_detach(&self, lease: &VolumeLease) -> Result<(), VolumeError>;

    /// Returns expired leases that require supervised recovery.
    ///
    /// Expiry is only a signal to begin recovery; it never permits immediate
    /// writable reuse.
    ///
    /// # Errors
    ///
    /// Returns an error when durable metadata cannot be read.
    async fn stale_leases(&self, now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError>;

    /// Fences an expired lease before provider cleanup begins.
    ///
    /// # Errors
    ///
    /// Returns an error if the supplied lease is no longer current.
    async fn begin_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError>;

    /// Releases a fenced lease after provider cleanup confirms detachment.
    ///
    /// # Errors
    ///
    /// Returns an error if the supplied lease is no longer current.
    async fn finish_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError>;
}
