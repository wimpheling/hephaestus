use async_trait::async_trait;
use runtime_types::{AgentInstanceId, LeaseId, RunId, VolumeId};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{
    INSTANCE_STATE_DISK_ID, Volume, VolumeAttachment, VolumeError, VolumeKind, VolumeLease,
    VolumeState, VolumeStore,
};

use super::helpers::lock;

pub struct MemoryVolumeStore {
    pub volume: Volume,
    pub lease: VolumeLease,
    pub log: Arc<StdMutex<Vec<&'static str>>>,
    pub stale: StdMutex<Vec<VolumeLease>>,
}

impl MemoryVolumeStore {
    pub fn new(instance_id: AgentInstanceId, log: Arc<StdMutex<Vec<&'static str>>>) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            volume: Volume {
                id: VolumeId::new(),
                instance_id,
                kind: VolumeKind::InstanceState,
                host_id: String::from("test"),
                host_path: PathBuf::from("/fake/agent-state.raw"),
                capacity_bytes: 32 * 1024 * 1024,
                filesystem_uuid: Uuid::new_v4(),
                state: VolumeState::Ready,
                key_reference: None,
                encryption_version: None,
                backup_revision: None,
                checksum: None,
                last_successful_backup_at: None,
            },
            lease: VolumeLease {
                id: LeaseId::new(),
                volume_id: VolumeId::new(),
                run_id: RunId::new(),
                host_id: String::from("test"),
                fencing_token: 1,
                acquired_at: now,
                heartbeat_at: now,
                expires_at: now + time::Duration::minutes(1),
                attached_at: None,
            },
            log,
            stale: StdMutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl VolumeStore for MemoryVolumeStore {
    async fn resolve_instance_state(
        &self,
        _agent_id: AgentInstanceId,
        _capacity_bytes: u64,
    ) -> Result<Volume, VolumeError> {
        Ok(self.volume.clone())
    }

    async fn acquire(
        &self,
        volume_id: VolumeId,
        run_id: RunId,
    ) -> Result<VolumeAttachment, VolumeError> {
        let mut lease = self.lease.clone();
        lease.volume_id = volume_id;
        lease.run_id = run_id;
        Ok(VolumeAttachment {
            volume: self.volume.clone(),
            lease,
            disk_id: INSTANCE_STATE_DISK_ID,
        })
    }

    async fn mark_attached(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        lock(&self.log).push("attached");
        let mut attached = lease.clone();
        attached.attached_at = Some(OffsetDateTime::now_utc());
        Ok(attached)
    }

    async fn heartbeat(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        Ok(lease.clone())
    }

    async fn active_lease_for_run(
        &self,
        run_id: RunId,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        Ok(lock(&self.stale)
            .iter()
            .find(|lease| lease.run_id == run_id)
            .cloned())
    }

    async fn release_after_detach(&self, _lease: &VolumeLease) -> Result<(), VolumeError> {
        lock(&self.log).push("release");
        Ok(())
    }

    async fn stale_leases(&self, _now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        Ok(lock(&self.stale).clone())
    }

    async fn begin_recovery(&self, _lease: &VolumeLease) -> Result<(), VolumeError> {
        lock(&self.log).push("recover-begin");
        Ok(())
    }

    async fn finish_recovery(&self, _lease: &VolumeLease) -> Result<(), VolumeError> {
        lock(&self.log).push("recover-finish");
        Ok(())
    }
}
