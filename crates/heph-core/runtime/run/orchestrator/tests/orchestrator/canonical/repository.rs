use async_trait::async_trait;
use run_domain::{
    Run, RunCleanupHostId, RunCleanupLeaseFence, RunCleanupReceipt, RunCleanupTarget,
    RunCleanupVmTarget, RunState,
};
use run_orchestrator::{RepositoryError, RunCleanupRepository, RunRepository};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Mutex as AsyncMutex;
use vm_trait::VmId;

use super::super::support::{MemoryRepository, MemoryVolumeStore, lock};
use super::provider::Pause;

#[derive(Default)]
pub struct CleanupState {
    pub binding: Option<(RunCleanupHostId, VmId)>,
    pub target: Option<RunCleanupTarget>,
    pub receipt: Option<RunCleanupReceipt>,
    pub completed: bool,
}

pub struct CleanupRepository {
    pub runs: Arc<MemoryRepository>,
    pub volumes: Arc<MemoryVolumeStore>,
    pub state: AsyncMutex<CleanupState>,
    pub log: Arc<Mutex<Vec<&'static str>>>,
    pub planning_calls: AtomicUsize,
    pub pause_planning: Mutex<Option<(usize, Arc<Pause>)>>,
}

#[async_trait]
impl RunCleanupRepository for CleanupRepository {
    async fn bind_vm_before_provision(
        &self,
        run_id: RunId,
        instance: AgentInstanceId,
        revision: AgentInstanceRevisionId,
        host: &RunCleanupHostId,
        vm: &VmId,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.lock().await;
        let durable_plan = self.runs.planned_vm.lock().await.clone();
        let mut run = self.runs.run.lock().await;
        if (run.id, run.instance_id, run.instance_revision_id) != (run_id, instance, revision)
            || state.target.is_some()
            || state
                .binding
                .as_ref()
                .is_some_and(|binding| binding != &(host.clone(), vm.clone()))
            || (state.binding.is_none()
                && run.vm_id.is_some()
                && durable_plan.as_ref() != Some(&(host.clone(), vm.clone())))
        {
            return Err(RepositoryError::InvalidData(
                "binding conflicts with durable history",
            ));
        }
        state.binding = Some((host.clone(), vm.clone()));
        run.vm_id = Some(vm.0.clone());
        drop(run);
        drop(state);
        lock(&self.log).push("bind-planned");
        let call = self.planning_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let pause = lock(&self.pause_planning)
            .as_ref()
            .filter(|(at, _)| *at == call)
            .map(|(_, pause)| pause.clone());
        if let Some(pause) = pause {
            pause.wait().await;
        }
        Ok(())
    }

    async fn begin_cleanup(&self, run_id: RunId) -> Result<RunCleanupTarget, RepositoryError> {
        let mut state = self.state.lock().await;
        if let Some(target) = state.target.as_ref() {
            return Ok(target.clone());
        }
        let run = self.runs.get(run_id).await?;
        if state.binding.is_none() {
            let plan = self.runs.planned_vm.lock().await;
            state.binding.clone_from(&plan);
            drop(plan);
        }
        let vm = state
            .binding
            .as_ref()
            .map_or_else(RunCleanupVmTarget::historical_unresolved, |(host, id)| {
                RunCleanupVmTarget::planned(host.clone(), id.clone()).unwrap()
            });
        let leases = lock(&self.volumes.stale)
            .iter()
            .filter(|lease| lease.run_id == run_id)
            .map(|lease| {
                RunCleanupLeaseFence::new(
                    run_id,
                    lease.id,
                    lease.volume_id,
                    lease.host_id.clone(),
                    lease.fencing_token,
                )
                .unwrap()
            })
            .collect();
        let target = RunCleanupTarget::new(
            run_id,
            run.instance_id,
            run.instance_revision_id,
            vm,
            1,
            leases,
        )
        .unwrap();
        state.target = Some(target.clone());
        drop(state);
        lock(&self.log).push("freeze");
        Ok(target)
    }

    async fn recorded_cleanup_receipt(
        &self,
        _run_id: RunId,
    ) -> Result<Option<RunCleanupReceipt>, RepositoryError> {
        Ok(self.state.lock().await.receipt.clone())
    }

    async fn record_cleanup_receipt(
        &self,
        receipt: &RunCleanupReceipt,
    ) -> Result<(), RepositoryError> {
        let mut state = self.state.lock().await;
        if state.target.as_ref() != Some(receipt.target())
            || state.receipt.as_ref().is_some_and(|old| old != receipt)
        {
            return Err(RepositoryError::InvalidData(
                "receipt conflicts with frozen target",
            ));
        }
        state.receipt = Some(receipt.clone());
        drop(state);
        lock(&self.log).push("receipt");
        Ok(())
    }

    async fn finish_cleanup(&self, receipt: &RunCleanupReceipt) -> Result<Run, RepositoryError> {
        let mut state = self.state.lock().await;
        if state.receipt.as_ref() != Some(receipt) {
            return Err(RepositoryError::InvalidData("finish lacks durable receipt"));
        }
        if state.completed {
            return self.runs.get(receipt.target().run_id()).await;
        }
        let mut run = self.runs.run.lock().await;
        if run.state != RunState::CleaningUp {
            return Err(RepositoryError::InvalidData("finish lacks cleaning state"));
        }
        lock(&self.volumes.stale).retain(|lease| lease.run_id != run.id);
        run.state = RunState::CleanedUp;
        let cleaned = run.clone();
        drop(run);
        state.completed = true;
        drop(state);
        lock(&self.log).push("finish-all");
        Ok(cleaned)
    }
}
