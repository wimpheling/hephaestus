//! Actual metadata operations with controlled VM/provider effects only.

use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    INSTANCE_STATE_DISK_ID, ScalarLeaseHistory, Volume, VolumeAttachment, VolumeError, VolumeLease,
    VolumeMetadataRepository, VolumeStore,
};

pub struct ControlledVolumes {
    pub metadata: PostgresVolumeMetadataRepository,
    pub volume: Option<VolumeId>,
    pub log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl VolumeStore for ControlledVolumes {
    async fn resolve_instance_state(
        &self,
        instance: AgentInstanceId,
        _capacity: u64,
    ) -> Result<Volume, VolumeError> {
        let volume = self
            .metadata
            .volume(
                self.volume
                    .ok_or(VolumeError::InvalidState("controlled state volume missing"))?,
            )
            .await?;
        assert_eq!(volume.instance_id, Some(instance));
        Ok(volume)
    }

    async fn acquire(&self, volume: VolumeId, run: RunId) -> Result<VolumeAttachment, VolumeError> {
        let now = OffsetDateTime::now_utc();
        let lease = self
            .metadata
            .acquire(
                volume,
                run,
                "lifecycle-host",
                now,
                now + time::Duration::minutes(5),
            )
            .await?;
        Ok(VolumeAttachment {
            volume: self.metadata.volume(volume).await?,
            lease,
            disk_id: INSTANCE_STATE_DISK_ID,
        })
    }

    async fn mark_attached(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        let now = OffsetDateTime::now_utc();
        self.metadata
            .mark_attached(lease, now, now + time::Duration::minutes(5))
            .await
    }

    async fn heartbeat(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        let now = OffsetDateTime::now_utc();
        self.metadata
            .heartbeat(lease, now, now + time::Duration::minutes(5))
            .await
    }

    async fn scalar_lease_history(&self, run: RunId) -> Result<ScalarLeaseHistory, VolumeError> {
        self.metadata.scalar_lease_history(run).await
    }

    async fn active_lease_for_run(&self, run: RunId) -> Result<Option<VolumeLease>, VolumeError> {
        let _ = run;
        Ok(None) // The core must reload the global original tuple, not this cache.
    }

    async fn release_after_detach(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        assert!(
            self.log.lock().unwrap().contains(&"scoped-confirmed"),
            "scoped provider confirmation precedes any release"
        );
        self.metadata.release_after_detach(lease, false).await?;
        self.log.lock().unwrap().push("release");
        Ok(())
    }

    async fn stale_leases(&self, _: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        panic!("strict runtime must not sweep global leases")
    }
    async fn begin_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        self.metadata.begin_recovery(lease).await
    }
    async fn finish_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        assert!(self.log.lock().unwrap().contains(&"scoped-confirmed"));
        self.metadata.release_after_detach(lease, true).await?;
        self.log.lock().unwrap().push("recover-release");
        Ok(())
    }
}
