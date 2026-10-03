use async_trait::async_trait;
use runtime_types::{LeaseId, RunId, VolumeId};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use volume_domain::{
    RunVolumeIdentity, RunVolumeSelection, RunVolumeSelections, VolumeMountScope,
    VolumeSelectionOrigin, VolumeSlotDeclaration,
};
use volume_trait::{RunVolumeAttachment, RunVolumeLease, RunVolumeStore, VolumeError};

use super::super::super::support::{MemoryVolumeStore, lock};
use super::super::{Fixture, repository::CleanupRepository};

pub struct Store {
    pub selected: RunVolumeSelections,
    pub attachments: Vec<RunVolumeAttachment>,
    pub scalar: Arc<MemoryVolumeStore>,
    pub cleanup: Arc<CleanupRepository>,
    pub incomplete: AtomicBool,
    pub wrong_release: AtomicBool,
    pub mount_denied: AtomicBool,
    pub foreign: AtomicBool,
    pub changed_fence: AtomicBool,
    pub heartbeats: AtomicUsize,
    pub attached: Notify,
}

impl Store {
    pub fn new(fixture: &Fixture, count: usize) -> Self {
        let command = &fixture.command;
        let identity = RunVolumeIdentity::new(
            command.run_id,
            command.instance_id,
            command.instance_revision_id,
            command.release_id,
            command.release_agent_id,
            fixture.volumes.volume.project_id,
        )
        .unwrap();
        let mut selections = Vec::new();
        let mut attachments = Vec::new();
        for index in 0..count {
            let volume_id = VolumeId::new();
            let slot = format!("data{index:02}");
            let scope: VolumeMountScope = serde_json::from_value(serde_json::json!({
                "instance_id":command.instance_id, "revision_id":command.instance_revision_id,
                "release_agent_id":command.release_agent_id, "slot":slot, "volume_id":volume_id,
                "access_mode":"read_write", "release_contract_hash": vec![7u8;32],
            }))
            .unwrap();
            let declaration: VolumeSlotDeclaration = serde_json::from_value(serde_json::json!({
                "slot":slot, "guest_path":format!("/data{index:02}"), "access_mode":"read_write",
                "required":true, "minimum_capacity_bytes":1,
            }))
            .unwrap();
            let selection = RunVolumeSelection::new(
                identity,
                scope,
                declaration,
                VolumeSelectionOrigin::Explicit,
            )
            .unwrap();
            let mut volume = fixture.volumes.volume.clone();
            volume.id = volume_id;
            volume.instance_id = None;
            volume.filesystem_uuid = uuid::Uuid::new_v4();
            let mut lease = fixture.volumes.lease.clone();
            lease.id = LeaseId::new();
            lease.run_id = command.run_id;
            lease.volume_id = volume_id;
            attachments.push(RunVolumeAttachment {
                volume,
                lease: RunVolumeLease::selected(lease, selection.clone()).unwrap(),
                disk_id: slot,
            });
            selections.push(selection);
        }
        Self {
            selected: RunVolumeSelections::new(identity, selections).unwrap(),
            attachments,
            scalar: fixture.volumes.clone(),
            cleanup: fixture.cleanup.clone(),
            incomplete: AtomicBool::new(false),
            wrong_release: AtomicBool::new(false),
            mount_denied: AtomicBool::new(false),
            foreign: AtomicBool::new(false),
            changed_fence: AtomicBool::new(false),
            heartbeats: AtomicUsize::new(0),
            attached: Notify::new(),
        }
    }

    pub fn legacy(fixture: &Fixture) -> Self {
        let mut store = Self::new(fixture, 1);
        let initial = &store.attachments[0];
        let mut scope = serde_json::to_value(initial.lease.selection().unwrap().scope()).unwrap();
        scope["slot"] = serde_json::json!("state");
        let declaration = serde_json::from_value(serde_json::json!({
            "slot":"state", "guest_path":"/var/lib/hephaestus", "access_mode":"read_write",
            "required":true, "minimum_capacity_bytes":1,
        }))
        .unwrap();
        let selection = RunVolumeSelection::new(
            store.selected.identity(),
            serde_json::from_value(scope).unwrap(),
            declaration,
            VolumeSelectionOrigin::LegacyOrigin,
        )
        .unwrap();
        store.selected =
            RunVolumeSelections::new(store.selected.identity(), vec![selection.clone()]).unwrap();
        let attachment = &mut store.attachments[0];
        attachment.volume.instance_id = Some(fixture.command.instance_id);
        attachment.disk_id = volume_trait::INSTANCE_STATE_DISK_ID.into();
        attachment.lease =
            RunVolumeLease::selected(attachment.lease.lease().clone(), selection).unwrap();
        store
    }

    fn leases(&self) -> Vec<RunVolumeLease> {
        self.attachments
            .iter()
            .map(|item| {
                let mut lease = item.lease.lease().clone();
                if self.changed_fence.load(Ordering::SeqCst) {
                    lease.fencing_token += 1;
                }
                RunVolumeLease::selected(lease, item.lease.selection().unwrap().clone()).unwrap()
            })
            .collect()
    }
}

#[async_trait]
impl RunVolumeStore for Store {
    async fn load_run_selections(&self, _run: RunId) -> Result<RunVolumeSelections, VolumeError> {
        if self.wrong_release.load(Ordering::SeqCst) {
            let value = self.selected.identity();
            let identity = RunVolumeIdentity::new(
                value.run_id(),
                value.instance_id(),
                value.revision_id(),
                runtime_types::ReleaseId::new(),
                value.release_agent_id(),
                value.project_id(),
            )
            .unwrap();
            return Ok(RunVolumeSelections::new(identity, vec![]).unwrap());
        }
        Ok(self.selected.clone())
    }
    async fn preflight_run(&self, selected: &RunVolumeSelections) -> Result<(), VolumeError> {
        if self.mount_denied.load(Ordering::SeqCst)
            || selected != &self.selected
            || self.cleanup.state.lock().await.target.is_some()
        {
            return Err(VolumeError::InvalidState("closed or mismatched selections"));
        }
        Ok(())
    }
    async fn acquire_run(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeAttachment>, VolumeError> {
        self.preflight_run(selected).await?;
        *lock(&self.scalar.stale) = self
            .attachments
            .iter()
            .map(|item| item.lease.lease().clone())
            .collect();
        let mut result = self.attachments.clone();
        if self.incomplete.load(Ordering::SeqCst) {
            result.pop();
        }
        if self.foreign.load(Ordering::SeqCst) {
            let attachment = result.first_mut().unwrap();
            let mut lease = attachment.lease.lease().clone();
            lease.host_id = "foreign".into();
            attachment.lease =
                RunVolumeLease::selected(lease, attachment.lease.selection().unwrap().clone())?;
        }
        Ok(result)
    }
    async fn mark_run_attached(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        self.preflight_run(selected).await?;
        self.attached.notify_one();
        Ok(self.leases())
    }
    async fn heartbeat_run(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        self.heartbeats.fetch_add(1, Ordering::SeqCst);
        self.preflight_run(selected).await?;
        Ok(self.leases())
    }
    async fn leases_for_run(&self, run: RunId) -> Result<Vec<RunVolumeLease>, VolumeError> {
        lock(&self.scalar.stale)
            .iter()
            .filter(|lease| lease.run_id == run)
            .cloned()
            .map(RunVolumeLease::historical_unproven)
            .collect()
    }
}
