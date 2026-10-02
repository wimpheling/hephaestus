use std::collections::BTreeSet;

use run_domain::Run;
use volume_domain::{RunVolumeSelections, VolumeSelectionOrigin};
use volume_trait::{RunVolumeAttachment, RunVolumeLease, VolumeAttachment, VolumeError};

/// Immutable acquired evidence; refreshed timestamps may change, fences may not.
#[derive(Clone)]
pub struct PreparedRunVolumes {
    pub selections: RunVolumeSelections,
    pub attachments: Vec<RunVolumeAttachment>,
    pub leases: Vec<RunVolumeLease>,
}

impl PreparedRunVolumes {
    pub fn new(
        run: &Run,
        selections: RunVolumeSelections,
        attachments: Vec<RunVolumeAttachment>,
        host: &str,
    ) -> Result<Self, VolumeError> {
        validate_identity(run, &selections)?;
        let leases = attachments.iter().map(|item| item.lease.clone()).collect();
        let result = Self {
            selections,
            attachments,
            leases,
        };
        result.validate_leases(&result.leases, host)?;
        let mut disks = BTreeSet::new();
        let mut filesystems = BTreeSet::new();
        for attachment in &result.attachments {
            let selected = attachment.lease.selection().ok_or(invalid())?;
            let volume = &attachment.volume;
            if volume.id != selected.scope().volume_id()
                || volume.project_id != result.selections.identity().project_id()
                || volume.host_id != host
                || volume.capacity_bytes < selected.declaration().minimum_capacity_bytes()
                || volume.filesystem_uuid.is_nil()
                || !disks.insert(&attachment.disk_id)
                || !filesystems.insert(volume.filesystem_uuid)
                || (selected.origin() == VolumeSelectionOrigin::LegacyOrigin
                    && volume.instance_id != Some(run.instance_id))
            {
                return Err(invalid());
            }
        }
        Ok(result)
    }

    pub fn legacy_attachment(&self) -> Option<VolumeAttachment> {
        // Domain validation permits LegacyOrigin only as an exact singleton.
        let selected = self.selections.selections().first()?;
        if selected.origin() != VolumeSelectionOrigin::LegacyOrigin {
            return None;
        }
        let attachment = self
            .attachments
            .iter()
            .find(|item| item.lease.selection() == Some(selected))?;
        Some(VolumeAttachment {
            volume: attachment.volume.clone(),
            lease: attachment.lease.lease().clone(),
            disk_id: volume_trait::INSTANCE_STATE_DISK_ID,
        })
    }

    pub fn validate_leases(
        &self,
        leases: &[RunVolumeLease],
        host: &str,
    ) -> Result<(), VolumeError> {
        if leases.len() != self.selections.selections().len() {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        for selection in self.selections.selections() {
            let mut matches = leases
                .iter()
                .filter(|lease| lease.selection() == Some(selection));
            let lease = matches.next().ok_or(invalid())?.lease();
            if matches.next().is_some() || lease.host_id != host || !ids.insert(lease.id) {
                return Err(invalid());
            }
        }
        Ok(())
    }

    pub fn refreshed(
        &mut self,
        leases: Vec<RunVolumeLease>,
        host: &str,
    ) -> Result<(), VolumeError> {
        self.validate_leases(&leases, host)?;
        for original in &self.leases {
            let actual = leases
                .iter()
                .find(|item| item.selection() == original.selection())
                .ok_or(invalid())?
                .lease();
            let expected = original.lease();
            if actual.id != expected.id
                || actual.volume_id != expected.volume_id
                || actual.run_id != expected.run_id
                || actual.host_id != expected.host_id
                || actual.fencing_token != expected.fencing_token
                || actual.acquired_at != expected.acquired_at
            {
                return Err(invalid());
            }
        }
        self.leases = leases;
        Ok(())
    }
}

pub fn validate_identity(run: &Run, selections: &RunVolumeSelections) -> Result<(), VolumeError> {
    let identity = selections.identity();
    if identity.run_id() != run.id
        || identity.instance_id() != run.instance_id
        || identity.revision_id() != run.instance_revision_id
        || identity.release_id() != run.release_id
        || identity.release_agent_id() != run.release_agent_id
    {
        return Err(invalid());
    }
    Ok(())
}

const fn invalid() -> VolumeError {
    VolumeError::InvalidState(
        "complete volume evidence differs from exact run or acquired fence set",
    )
}
