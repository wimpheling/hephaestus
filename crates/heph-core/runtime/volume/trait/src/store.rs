//! Persistent volume operations used by the run orchestrator.

use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use time::OffsetDateTime;

use crate::{ScalarLeaseHistory, Volume, VolumeAttachment, VolumeError, VolumeLease};

/// Persistent volume operations used by the run orchestrator.
#[async_trait]
pub trait VolumeStore: Send + Sync + 'static {
    /// Creates or resolves the single instance-state volume for `instance_id`.
    ///
    /// The returned backing file is formatted by the host before the volume
    /// enters [`VolumeState::Ready`](crate::VolumeState::Ready).
    ///
    /// # Errors
    ///
    /// Returns an error when metadata or backing-file creation fails.
    async fn resolve_instance_state(
        &self,
        instance_id: AgentInstanceId,
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

    /// Observes global original scalar history; absence is never host-filtered.
    ///
    /// # Errors
    ///
    /// Unsupported adapters and contradictory or multiple history fail closed.
    async fn scalar_lease_history(
        &self,
        _run_id: RunId,
    ) -> Result<ScalarLeaseHistory, VolumeError> {
        Err(VolumeError::InvalidState(
            "global scalar history is unsupported",
        ))
    }

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
