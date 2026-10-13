//! Additive exact-run ports implemented by the existing metadata and local stores.

use async_trait::async_trait;
use runtime_types::RunId;
use time::OffsetDateTime;
use volume_domain::{RunVolumeSelection, RunVolumeSelections};

use crate::{Volume, VolumeError, VolumeLease};

/// Canonical lease with immutable selection evidence, or unproven historical evidence.
///
/// Unproven rows may be recovered after confirmed destruction. They are never
/// an input to acquisition and never authorize attachment or continued execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunVolumeLease {
    lease: VolumeLease,
    selection: Option<RunVolumeSelection>,
}

impl RunVolumeLease {
    /// Reconstructs a selected canonical row without substituting its run or fence.
    ///
    /// # Errors
    ///
    /// Rejects nonpositive fences and mismatched run or resource identities.
    /// The repository must prove persisted provenance and current authority.
    pub fn selected(
        lease: VolumeLease,
        selection: RunVolumeSelection,
    ) -> Result<Self, VolumeError> {
        validate_lease(&lease)?;
        if lease.run_id != selection.identity().run_id()
            || lease.volume_id != selection.scope().volume_id()
        {
            return Err(VolumeError::InvalidState(
                "lease differs from exact run selection",
            ));
        }
        Ok(Self {
            lease,
            selection: Some(selection),
        })
    }

    /// Reconstructs an unmatched historical row for recovery only.
    ///
    /// # Errors
    ///
    /// Rejects nil identities and nonpositive fences. This constructor does not
    /// grant access and cannot be supplied to the acquisition port.
    pub fn historical_unproven(lease: VolumeLease) -> Result<Self, VolumeError> {
        validate_lease(&lease)?;
        Ok(Self {
            lease,
            selection: None,
        })
    }

    /// Exact canonical lease ID, host, run and fencing generation.
    #[must_use]
    pub const fn lease(&self) -> &VolumeLease {
        &self.lease
    }
    /// Exact selected immutable scope; absent only for recovery-only history.
    #[must_use]
    pub const fn selection(&self) -> Option<&RunVolumeSelection> {
        self.selection.as_ref()
    }
}

const fn validate_lease(lease: &VolumeLease) -> Result<(), VolumeError> {
    if lease.id.as_uuid().is_nil()
        || lease.run_id.as_uuid().is_nil()
        || lease.volume_id.as_uuid().is_nil()
        || lease.fencing_token <= 0
    {
        return Err(VolumeError::InvalidState(
            "lease evidence has invalid identity or fence",
        ));
    }
    Ok(())
}

/// Provider-internal attachment for one exact named or proven legacy selection.
///
/// Disk identity is derived from the exact slot by the trusted store; the raw
/// backing path remains in provider-internal volume metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunVolumeAttachment {
    /// Canonical resource and its provider-owned backing handles.
    pub volume: Volume,
    /// Exact selected lease, never unproven historical evidence.
    pub lease: RunVolumeLease,
    /// Unique controlled block-device identity for this selected slot.
    pub disk_id: String,
}

/// Worker-only extension of the existing canonical volume metadata repository.
///
/// Implementations use the existing volume, binding, run and lease tables. No
/// declaration, historical snapshot or caller-supplied value grants authority.
#[async_trait]
pub trait RunVolumeMetadataRepository: Send + Sync + 'static {
    /// Loads bounded selections from the run's exact stored revision and release.
    ///
    /// Rejects unsupported named dispatch, incomplete required bindings, invalid
    /// projects/declarations, and permanent closure. Legacy origins are proved
    /// from the actual release and same-origin scalar pointer, not a command flag.
    async fn load_run_selections(&self, run_id: RunId) -> Result<RunVolumeSelections, VolumeError>;

    /// Rechecks the complete selected set and every live source/grant before effects.
    ///
    /// Mount-grant source checks are separate from the caller's release-use and
    /// attachment-execute or update authority. Orchestration must check those
    /// caller permissions before every acquire, launch and heartbeat.
    ///
    /// Orchestration must enforce one combined authorization deadline and destroy
    /// the VM on denial/error/timeout; this port does not claim a physical deadline.
    async fn preflight_run(&self, selections: &RunVolumeSelections) -> Result<(), VolumeError>;

    /// Atomically acquires the complete set, initially exclusive even for read-only.
    ///
    /// Locks consumer/run before volumes in global resource-ID order, proves all
    /// declarations and live authorities before inserting any lease, and commits
    /// every selection/fence/run identity before returning in slot order. Exact retries return
    /// the same rows. Closed cleanup or changed selections reject without effects.
    async fn acquire_run(
        &self,
        selections: &RunVolumeSelections,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError>;

    /// Marks the complete exact lease set attached after launch confirmation.
    ///
    /// Revalidates selection equality, all live authorities and run admission in
    /// one transaction; partial, historical or changed lease sets cannot attach.
    async fn mark_run_attached(
        &self,
        selections: &RunVolumeSelections,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError>;

    /// Rechecks all current sources and refreshes the complete exact lease set.
    ///
    /// Authority denial or uncertain checks fail without extending any member.
    /// Expiry alone never proves detachment or permits reuse.
    async fn heartbeat_run(
        &self,
        selections: &RunVolumeSelections,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError>;

    /// Lists the complete active/recovery evidence in deterministic slot/ID order.
    ///
    /// Historical unmatched rows are explicitly recovery-only. Empty is an
    /// explicit result; no singular projection is inferred from the first row.
    /// Query globally by run, then reject if any lease belongs to another host;
    /// never hide foreign-host resources by filtering them out of the result.
    async fn leases_for_run(
        &self,
        run_id: RunId,
        host_id: &str,
    ) -> Result<Vec<RunVolumeLease>, VolumeError>;
}

/// Exact-run extension implemented by the existing local volume store.
///
/// Missing exact-run metadata composition fails unsupported before any named
/// lease or backing effect. Existing singular methods remain legacy-only.
#[async_trait]
pub trait RunVolumeStore: Send + Sync + 'static {
    /// Loads the authoritative complete selection set without backing effects.
    async fn load_run_selections(&self, run_id: RunId) -> Result<RunVolumeSelections, VolumeError>;
    /// Revalidates every selected mount before acquisition, launch or heartbeat.
    async fn preflight_run(&self, selections: &RunVolumeSelections) -> Result<(), VolumeError>;
    /// Acquires and verifies the complete set, retaining durable evidence on failure.
    ///
    /// Known owned ready RW files permit normal journal replay under the exclusive
    /// lease; dirty RO files fail. Ambiguous provisioning is never reformatted.
    async fn acquire_run(
        &self,
        selections: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeAttachment>, VolumeError>;
    /// Marks the complete exact set attached, rechecking every live authority.
    async fn mark_run_attached(
        &self,
        selections: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError>;
    /// Reauthorizes and refreshes the complete set; denial requires VM destruction.
    async fn heartbeat_run(
        &self,
        selections: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError>;
    /// Lists all canonical active/recovery leases for deterministic compensation.
    async fn leases_for_run(&self, run_id: RunId) -> Result<Vec<RunVolumeLease>, VolumeError>;
}
