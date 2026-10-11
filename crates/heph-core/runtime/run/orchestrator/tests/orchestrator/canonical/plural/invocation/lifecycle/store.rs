use async_trait::async_trait;
use run_domain::RunState;
use runtime_types::RunId;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volume_domain::RunVolumeSelections;
use volume_trait::{RunVolumeAttachment, RunVolumeLease, RunVolumeStore, VolumeError};

use super::Authority;
use crate::canonical::plural::Store;
use crate::support::lock;

pub struct CheckedStore {
    pub inner: Arc<Store>,
    pub authority: Arc<Authority>,
    pub partial_attachment: AtomicBool,
}
#[async_trait]
impl RunVolumeStore for CheckedStore {
    async fn load_run_selections(&self, run: RunId) -> Result<RunVolumeSelections, VolumeError> {
        self.inner.load_run_selections(run).await
    }
    async fn preflight_run(&self, selected: &RunVolumeSelections) -> Result<(), VolumeError> {
        self.inner.preflight_run(selected).await
    }
    async fn acquire_run(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeAttachment>, VolumeError> {
        self.inner.acquire_run(selected).await
    }
    async fn mark_run_attached(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        assert!(self.authority.acknowledged.load(Ordering::SeqCst));
        assert_eq!(
            self.authority.runs.run.lock().await.state,
            RunState::Provisioning
        );
        lock(&self.authority.log).push("current-full-attach");
        let mut result = self.inner.mark_run_attached(selected).await?;
        if self.partial_attachment.load(Ordering::SeqCst) {
            result.pop();
        }
        Ok(result)
    }
    async fn heartbeat_run(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        self.inner.heartbeat_run(selected).await
    }
    async fn leases_for_run(&self, run: RunId) -> Result<Vec<RunVolumeLease>, VolumeError> {
        self.inner.leases_for_run(run).await
    }
}
