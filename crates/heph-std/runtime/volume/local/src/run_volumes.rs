//! Exact-run operations, explicitly composed and never used as legacy fallback.

use async_trait::async_trait;
use runtime_types::RunId;
use time::OffsetDateTime;
use volume_domain::RunVolumeSelections;
use volume_trait::{
    RunVolumeAttachment, RunVolumeLease, RunVolumeMetadataRepository, RunVolumeStore, VolumeError,
};

use crate::LocalVolumeStore;

impl LocalVolumeStore {
    fn exact_run_metadata(&self) -> Result<&dyn RunVolumeMetadataRepository, VolumeError> {
        self.run_metadata
            .as_deref()
            .ok_or(VolumeError::InvalidState(
                "exact-run volume metadata is not configured",
            ))
    }
}

#[async_trait]
impl RunVolumeStore for LocalVolumeStore {
    async fn load_run_selections(&self, run_id: RunId) -> Result<RunVolumeSelections, VolumeError> {
        self.exact_run_metadata()?.load_run_selections(run_id).await
    }

    async fn preflight_run(&self, selections: &RunVolumeSelections) -> Result<(), VolumeError> {
        self.exact_run_metadata()?.preflight_run(selections).await
    }

    async fn acquire_run(
        &self,
        selections: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeAttachment>, VolumeError> {
        let now = OffsetDateTime::now_utc();
        let leases = self
            .exact_run_metadata()?
            .acquire_run(selections, &self.config.host_id, now, self.expiry(now)?)
            .await?;
        if leases.len() != selections.selections().len()
            || leases
                .iter()
                .zip(selections.selections())
                .any(|(lease, selected)| lease.selection() != Some(selected))
        {
            return Err(VolumeError::InvalidState(
                "metadata returned a different complete selection set",
            ));
        }
        let mut attachments = Vec::with_capacity(leases.len());
        for lease in leases {
            let selection = lease.selection().ok_or(VolumeError::InvalidState(
                "historical evidence cannot attach",
            ))?;
            let volume = self.metadata.volume(lease.lease().volume_id).await?;
            if volume.project_id != selection.identity().project_id()
                || volume.id != selection.scope().volume_id()
                || volume.capacity_bytes < selection.declaration().minimum_capacity_bytes()
            {
                return Err(VolumeError::IntentConflict);
            }
            // All canonical leases are committed already. A backing failure
            // retains every lease for deterministic cleanup, never an unproved release.
            self.verify_run_backing(&volume, selection.scope().access_mode())?;
            let disk_id = format!("volume-{}", selection.scope().slot().as_str());
            attachments.push(RunVolumeAttachment {
                volume,
                lease,
                disk_id,
            });
        }
        Ok(attachments)
    }

    async fn mark_run_attached(
        &self,
        selections: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        let now = OffsetDateTime::now_utc();
        self.exact_run_metadata()?
            .mark_run_attached(selections, &self.config.host_id, now, self.expiry(now)?)
            .await
    }

    async fn heartbeat_run(
        &self,
        selections: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        let now = OffsetDateTime::now_utc();
        self.exact_run_metadata()?
            .heartbeat_run(selections, &self.config.host_id, now, self.expiry(now)?)
            .await
    }

    async fn leases_for_run(&self, run_id: RunId) -> Result<Vec<RunVolumeLease>, VolumeError> {
        self.exact_run_metadata()?
            .leases_for_run(run_id, &self.config.host_id)
            .await
    }
}
