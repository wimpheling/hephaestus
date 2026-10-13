use super::{
    super::support::{MemoryVolumeStore, lock},
    repository::Repository,
};
use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use volume_trait::{
    ScalarLeaseHistory, Volume, VolumeAttachment, VolumeError, VolumeLease, VolumeStore,
};

pub struct Volumes {
    pub inner: MemoryVolumeStore,
    pub runs: Arc<Repository>,
    pub history: Mutex<ScalarLeaseHistory>,
    pub fail_after_acquire: AtomicBool,
}
impl Volumes {
    pub async fn set_held(&self, run: RunId) -> VolumeLease {
        let mut lease = self.inner.lease.clone();
        lease.volume_id = self.inner.volume.id;
        lease.run_id = run;
        let mut row = self.runs.inner.run.lock().await;
        row.volume_id = Some(lease.volume_id);
        row.lease_id = Some(lease.id);
        row.lease_fencing_token = Some(lease.fencing_token);
        drop(row);
        *self.history.lock().await = ScalarLeaseHistory::Held(lease.clone());
        lease
    }
    async fn release(&self, lease: &VolumeLease, recovering: bool) -> Result<(), VolumeError> {
        let mut h = self.history.lock().await;
        let exact = match &*h {
            ScalarLeaseHistory::Held(existing) if !recovering => existing == lease,
            ScalarLeaseHistory::HeldRecovering(existing) if recovering => existing == lease,
            _ => false,
        };
        if !exact {
            return Err(VolumeError::StaleLease);
        }
        *h = ScalarLeaseHistory::Released(lease.clone());
        drop(h);
        lock(&self.inner.log).push(if recovering {
            "recover-finish"
        } else {
            "release"
        });
        Ok(())
    }
}
#[async_trait]
impl VolumeStore for Volumes {
    async fn resolve_instance_state(
        &self,
        i: AgentInstanceId,
        c: u64,
    ) -> Result<Volume, VolumeError> {
        self.inner.resolve_instance_state(i, c).await
    }
    async fn acquire(&self, v: VolumeId, r: RunId) -> Result<VolumeAttachment, VolumeError> {
        let a = self.inner.acquire(v, r).await?;
        self.set_held(r).await;
        if self.fail_after_acquire.load(Ordering::SeqCst) {
            return Err(VolumeError::InvalidState("IO after committed lease failed"));
        }
        Ok(a)
    }
    async fn mark_attached(&self, l: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        self.inner.mark_attached(l).await
    }
    async fn heartbeat(&self, l: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        self.inner.heartbeat(l).await
    }
    async fn scalar_lease_history(&self, _: RunId) -> Result<ScalarLeaseHistory, VolumeError> {
        Ok(self.history.lock().await.clone())
    }
    async fn active_lease_for_run(&self, _: RunId) -> Result<Option<VolumeLease>, VolumeError> {
        Ok(None)
    }
    async fn release_after_detach(&self, l: &VolumeLease) -> Result<(), VolumeError> {
        self.release(l, false).await
    }
    async fn stale_leases(&self, _: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        panic!("strict mode must not sweep global leases")
    }
    async fn begin_recovery(&self, l: &VolumeLease) -> Result<(), VolumeError> {
        *self.history.lock().await = ScalarLeaseHistory::HeldRecovering(l.clone());
        Ok(())
    }
    async fn finish_recovery(&self, l: &VolumeLease) -> Result<(), VolumeError> {
        self.release(l, true).await
    }
}
