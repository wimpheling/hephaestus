//! Existing complete-set store with an observable acquisition boundary.
use async_trait::async_trait;
use run_domain::RunState;
use runtime_types::RunId;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use vm_trait::VmProviderOwnerScope;
use volume_domain::RunVolumeSelections;
use volume_trait::{RunVolumeAttachment, RunVolumeLease, RunVolumeStore, VolumeError};

use crate::{
    canonical::{Fixture, plural::Store, provider::ScopedProvider},
    support::{MemoryRepository, lock},
};

pub struct CheckedStore {
    pub inner: Store,
    runs: Arc<MemoryRepository>,
    provider: Arc<ScopedProvider>,
    log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    /// Mutate after `prepare_start` captured its Run, before the helper's reread.
    pub substitute: AtomicU8,
}
impl CheckedStore {
    pub fn new(fixture: &Fixture, count: usize) -> Self {
        Self {
            inner: Store::new(fixture, count),
            runs: fixture.runs.clone(),
            provider: fixture.provider.clone(),
            log: fixture.log.clone(),
            substitute: AtomicU8::new(0),
        }
    }
}
#[async_trait]
impl RunVolumeStore for CheckedStore {
    async fn load_run_selections(&self, run: RunId) -> Result<RunVolumeSelections, VolumeError> {
        let selected = self.inner.load_run_selections(run).await?;
        let mode = self.substitute.load(Ordering::SeqCst);
        if mode == 1 {
            *lock(&self.provider.scope) =
                VmProviderOwnerScope::new("substituted-owner".into(), "test".into()).unwrap();
        } else if mode != 0 {
            let mut current = self.runs.run.lock().await;
            match mode {
                2 => current.state = RunState::Provisioning,
                3 => current.instance_revision_id = runtime_types::AgentInstanceRevisionId::new(),
                4 => current.release_id = runtime_types::ReleaseId::new(),
                5 => current.command_id = runtime_types::CommandId::new(),
                6 => current.requires_state = !current.requires_state,
                7 => current.vm_id = Some("substituted-vm".into()),
                8 => current.state_version += 1,
                _ => panic!("unknown substitution"),
            }
        }
        lock(&self.log).push("first-selection");
        Ok(selected)
    }
    async fn preflight_run(&self, selected: &RunVolumeSelections) -> Result<(), VolumeError> {
        lock(&self.log).push("first-mount");
        self.inner.preflight_run(selected).await
    }
    async fn acquire_run(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeAttachment>, VolumeError> {
        lock(&self.log).push("first-acquire");
        self.inner.acquire_run(selected).await
    }
    async fn mark_run_attached(
        &self,
        selected: &RunVolumeSelections,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        lock(&self.log).push("first-attach");
        self.inner.mark_run_attached(selected).await
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
