use async_trait::async_trait;
use runtime_types::RunId;
use std::sync::Arc;
use volume_domain::RunVolumeSelections;
use volume_trait::{RunVolumeAttachment, RunVolumeLease, RunVolumeStore, VolumeError};

use super::super::support::{MemoryVolumeStore, lock};

pub struct GlobalVolumes(pub Arc<MemoryVolumeStore>);

#[async_trait]
impl RunVolumeStore for GlobalVolumes {
    async fn load_run_selections(&self, _run: RunId) -> Result<RunVolumeSelections, VolumeError> {
        Err(VolumeError::InvalidState(
            "selection loading is outside this cleanup fixture",
        ))
    }
    async fn preflight_run(&self, _selected: &RunVolumeSelections) -> Result<(), VolumeError> {
        Err(VolumeError::InvalidState(
            "preflight is outside this cleanup fixture",
        ))
    }
    async fn acquire_run(
        &self,
        _selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeAttachment>, VolumeError> {
        Err(VolumeError::InvalidState(
            "plural acquisition is outside this cleanup fixture",
        ))
    }
    async fn mark_run_attached(
        &self,
        _selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        Err(VolumeError::InvalidState(
            "plural attachment is outside this cleanup fixture",
        ))
    }
    async fn heartbeat_run(
        &self,
        _selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        Err(VolumeError::InvalidState(
            "plural heartbeat is outside this cleanup fixture",
        ))
    }
    async fn leases_for_run(&self, run: RunId) -> Result<Vec<RunVolumeLease>, VolumeError> {
        lock(&self.0.stale)
            .iter()
            .filter(|lease| lease.run_id == run)
            .cloned()
            .map(RunVolumeLease::historical_unproven)
            .collect()
    }
}
