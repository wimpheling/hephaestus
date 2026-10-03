use super::super::support::MemoryRepository;
use async_trait::async_trait;
use run_domain::{
    CancelRun, LegacyVmPlacement, LegacyVmPlacementConsumption, LegacyVmPlacementInventory,
    LegacyVmPlacementProducer, LegacyVmPlacementScope, Run, RunState, StartRun,
};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{LeaseId, RunId, VolumeId};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use vm_trait::{VmExit, VmId};

pub struct Repository {
    pub inner: MemoryRepository,
    pub plan: Mutex<Option<LegacyVmPlacement>>,
    pub returned_close: Mutex<Option<LegacyVmPlacement>>,
    pub returned_consume: Mutex<Option<LegacyVmPlacement>>,
    pub closed: AtomicBool,
    pub fail_close: AtomicBool,
    pub hang_close: AtomicBool,
    pub held_inventory: AtomicBool,
    pub bad_inventory: AtomicBool,
    pub scope: LegacyVmPlacementScope,
    pub command: StartRun,
}
impl Repository {
    pub fn new(command: &StartRun, scope: LegacyVmPlacementScope) -> Self {
        Self {
            inner: MemoryRepository::new(command),
            plan: Mutex::new(None),
            returned_close: Mutex::new(None),
            returned_consume: Mutex::new(None),
            closed: AtomicBool::new(false),
            fail_close: AtomicBool::new(false),
            hang_close: AtomicBool::new(false),
            held_inventory: AtomicBool::new(false),
            bad_inventory: AtomicBool::new(false),
            scope,
            command: command.clone(),
        }
    }
    async fn projection(
        &self,
        scope: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        if scope != &self.scope {
            return Err(RepositoryError::InvalidData("wrong configured owner"));
        }
        self.plan
            .lock()
            .await
            .clone()
            .ok_or(RepositoryError::InvalidData("unproven history"))
    }
}
#[async_trait]
impl RunRepository for Repository {
    async fn create_run_with_legacy_placement(
        &self,
        c: &StartRun,
        s: &LegacyVmPlacementScope,
    ) -> Result<CreateRunResult, RepositoryError> {
        if c != &self.command || s != &self.scope {
            return Err(RepositoryError::InvalidData("wrong immutable pins"));
        }
        if self.closed.load(Ordering::SeqCst)
            && self.inner.run.lock().await.state != RunState::CleanedUp
        {
            return Err(RepositoryError::InvalidData("acquisition closed"));
        }
        let mut plan = self.plan.lock().await;
        if plan.is_none() {
            *plan = Some(
                LegacyVmPlacement::new(
                    c.clone(),
                    uuid::Uuid::new_v4(),
                    [7; 32],
                    s.clone(),
                    VmId(c.run_id.to_string()),
                    LegacyVmPlacementProducer::NormalRequest,
                    Some(LegacyVmPlacementConsumption::Provision),
                )
                .unwrap(),
            );
        }
        drop(plan);
        self.inner.create_run(c).await
    }
    async fn legacy_vm_placement(
        &self,
        _: RunId,
        s: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        self.projection(s).await
    }
    async fn assert_legacy_vm_placement_open(
        &self,
        r: RunId,
        s: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        let p = self.projection(s).await?;
        let state = self.inner.get(r).await?.state;
        if self.closed.load(Ordering::SeqCst)
            || p.consumption() != Some(LegacyVmPlacementConsumption::Provision)
            || !matches!(
                state,
                RunState::Queued | RunState::LeasingVolume | RunState::Provisioning
            )
        {
            return Err(RepositoryError::InvalidData("placement closed"));
        }
        Ok(p)
    }
    async fn close_legacy_vm_acquisition(
        &self,
        _: RunId,
        s: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        if self.hang_close.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        if self.fail_close.load(Ordering::SeqCst) {
            return Err(RepositoryError::InvalidData("DB closure unavailable"));
        }
        let p = self.projection(s).await?;
        self.closed.store(true, Ordering::SeqCst);
        Ok(self.returned_close.lock().await.clone().unwrap_or(p))
    }
    async fn consume_legacy_vm_placement(
        &self,
        c: &StartRun,
        s: &LegacyVmPlacementScope,
        k: LegacyVmPlacementConsumption,
    ) -> Result<LegacyVmPlacement, RepositoryError> {
        let p = self.projection(s).await?;
        if c != p.command()
            || !self.closed.load(Ordering::SeqCst)
            || k != LegacyVmPlacementConsumption::CleanupOnly
        {
            return Err(RepositoryError::InvalidData("invalid cleanup consumption"));
        }
        if p.consumption().is_some() {
            return Err(RepositoryError::InvalidData("one way consumption"));
        }
        let next = LegacyVmPlacement::new(
            c.clone(),
            p.project(),
            *p.contract_hash(),
            s.clone(),
            p.vm_id().clone(),
            p.producer(),
            Some(k),
        )
        .unwrap();
        *self.plan.lock().await = Some(next.clone());
        Ok(self.returned_consume.lock().await.clone().unwrap_or(next))
    }
    async fn legacy_vm_cleanup_completed(
        &self,
        r: RunId,
        s: &LegacyVmPlacementScope,
    ) -> Result<Run, RepositoryError> {
        let p = self.projection(s).await?;
        let run = self.inner.get(r).await?;
        if run.state != RunState::CleanedUp
            || !self.closed.load(Ordering::SeqCst)
            || p.consumption().is_none()
        {
            return Err(RepositoryError::InvalidData(
                "no qualified terminal evidence",
            ));
        }
        Ok(run)
    }
    async fn legacy_vm_placement_inventory(
        &self,
        s: &LegacyVmPlacementScope,
    ) -> Result<LegacyVmPlacementInventory, RepositoryError> {
        if self.held_inventory.load(Ordering::SeqCst) {
            return Ok(LegacyVmPlacementInventory {
                placements: vec![],
                closed_runs: vec![],
                held: vec![run_domain::LegacyVmPlacementHold {
                    run_id: self.command.run_id,
                    reason: run_domain::LegacyVmPlacementHoldReason::HistoricalUnproven,
                }],
            });
        }
        let p = self.projection(s).await?;
        Ok(LegacyVmPlacementInventory {
            placements: vec![p],
            held: vec![],
            closed_runs: if self.bad_inventory.load(Ordering::SeqCst) {
                vec![RunId::new()]
            } else if self.closed.load(Ordering::SeqCst) {
                vec![self.command.run_id]
            } else {
                vec![]
            },
        })
    }
    async fn create_run(&self, c: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        self.inner.create_run(c).await
    }
    async fn get(&self, r: RunId) -> Result<Run, RepositoryError> {
        self.inner.get(r).await
    }
    async fn ensure_runtime_git_provenance(&self, r: &Run) -> Result<(), RepositoryError> {
        self.inner.ensure_runtime_git_provenance(r).await
    }
    async fn bind_resources(
        &self,
        r: RunId,
        v: Option<VolumeId>,
        l: Option<LeaseId>,
        f: Option<i64>,
        vm: &str,
    ) -> Result<Run, RepositoryError> {
        self.assert_legacy_vm_placement_open(r, &self.scope).await?;
        if vm != self.command.run_id.to_string() {
            return Err(RepositoryError::InvalidData("foreign VM"));
        }
        self.inner.bind_resources(r, v, l, f, vm).await
    }
    async fn transition(
        &self,
        r: RunId,
        n: RunState,
        e: Option<&VmExit>,
        f: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        if matches!(
            n,
            RunState::LeasingVolume
                | RunState::Provisioning
                | RunState::Starting
                | RunState::Running
        ) && self.closed.load(Ordering::SeqCst)
        {
            return Err(RepositoryError::InvalidData("closed IO"));
        }
        if n == RunState::CleaningUp {
            self.closed.store(true, Ordering::SeqCst);
        }
        self.inner.transition(r, n, e, f).await
    }
    async fn append_vm_event(&self, r: RunId, e: StoredVmEvent) -> Result<(), RepositoryError> {
        self.inner.append_vm_event(r, e).await
    }
    async fn request_cancel(&self, c: &CancelRun) -> Result<bool, RepositoryError> {
        self.inner.request_cancel(c).await
    }
    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        panic!("strict mode must not use generic Run sweeps")
    }
}
