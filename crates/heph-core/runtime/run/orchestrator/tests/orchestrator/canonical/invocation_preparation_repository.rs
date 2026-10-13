//! Controlled prepared metadata; this adapter proves no SQL or physical absence.
use super::super::support::{
    MemoryRepository, RecordingCompletion, RecordingRuntimeManager, TestSpecFactory,
    TransitionPause,
};
use super::{Fixture, repository::CleanupRepository, volumes::GlobalVolumes};
use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{
    CreateRunResult, InvocationCancelEvidence, InvocationCancelPreparation,
    PreparedInvocationCancellation, RepositoryError, RunCleanupRepository, RunOrchestrator,
    RunRepository, StoredVmEvent,
};
use runtime_types::{CommandId, LeaseId, RunId, VolumeId};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex;
use vm_trait::{VmExit, VmProvider, VmProviderOwnerScope};

pub struct Repository {
    pub inner: Arc<MemoryRepository>,
    pub cleanup: Arc<CleanupRepository>,
    scope: VmProviderOwnerScope,
    pub queued: AtomicBool,
    pub unknown: AtomicBool,
    pub fail_preparation: AtomicBool,
    pub consumed_during_preparation: AtomicBool,
    pub substitute_revision: AtomicBool,
    pub classifications: AtomicUsize,
    pub preparations: AtomicUsize,
    pub pause_preparation: Mutex<Option<TransitionPause>>,
    controls: Mutex<HashSet<CommandId>>,
}

impl Repository {
    pub fn new(f: &Fixture, queued: bool) -> Arc<Self> {
        Arc::new(Self {
            inner: f.runs.clone(),
            cleanup: f.cleanup.clone(),
            scope: f.provider.owner_scope().unwrap(),
            queued: AtomicBool::new(queued),
            unknown: AtomicBool::new(false),
            fail_preparation: AtomicBool::new(false),
            consumed_during_preparation: AtomicBool::new(false),
            substitute_revision: AtomicBool::new(false),
            classifications: AtomicUsize::new(0),
            preparations: AtomicUsize::new(0),
            pause_preparation: Mutex::new(None),
            controls: Mutex::new(HashSet::new()),
        })
    }

    pub fn orchestrator(self: &Arc<Self>, f: &Fixture) -> RunOrchestrator {
        RunOrchestrator::new(
            self.clone(),
            f.volumes.clone(),
            f.provider.clone(),
            Arc::new(TestSpecFactory),
            32 * 1024 * 1024,
        )
        .with_cleanup_repository(
            f.cleanup.clone(),
            Arc::new(GlobalVolumes(f.volumes.clone())),
        )
        .with_qualified_invocation(self.scope.clone())
        .with_runtime_manager(Arc::new(RecordingRuntimeManager { log: f.log.clone() }))
        .with_completion_observer(Arc::new(RecordingCompletion { log: f.log.clone() }))
    }
}

#[async_trait]
impl RunRepository for Repository {
    async fn classify_invocation_cancel(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<InvocationCancelEvidence, RepositoryError> {
        self.classifications.fetch_add(1, Ordering::SeqCst);
        if scope != &self.scope
            || run != self.inner.run.lock().await.id
            || self.unknown.load(Ordering::SeqCst)
        {
            return Err(RepositoryError::InvalidData("controlled original is held"));
        }
        Ok(if self.queued.load(Ordering::SeqCst) {
            InvocationCancelEvidence::Queued115
        } else {
            InvocationCancelEvidence::Consumed117
        })
    }

    async fn prepare_invocation_cancel_cleanup(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<InvocationCancelPreparation, RepositoryError> {
        self.preparations.fetch_add(1, Ordering::SeqCst);
        if self
            .consumed_during_preparation
            .swap(false, Ordering::SeqCst)
        {
            // Controlled immutable classification race only; not a real117 receipt.
            self.queued.store(false, Ordering::SeqCst);
            return Ok(InvocationCancelPreparation::Consumed117);
        }
        if self.classify_invocation_cancel(run, scope).await?
            == InvocationCancelEvidence::Consumed117
        {
            return Ok(InvocationCancelPreparation::Consumed117);
        }
        if self.fail_preparation.load(Ordering::SeqCst) {
            return Err(RepositoryError::InvalidData(
                "controlled preparation failed",
            ));
        }
        let pause = self.pause_preparation.lock().await.clone();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            pause.release.notified().await;
        }
        let target = self.cleanup.begin_cleanup(run).await?;
        let mut prepared = self.inner.run.lock().await.clone();
        if self.substitute_revision.load(Ordering::SeqCst) {
            prepared.instance_revision_id = runtime_types::AgentInstanceRevisionId::new();
        }
        Ok(InvocationCancelPreparation::PreparedQueued(Box::new(
            PreparedInvocationCancellation {
                run: prepared,
                target,
            },
        )))
    }

    async fn create_run(&self, command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        self.inner.create_run(command).await
    }
    async fn create_run_with_vm_plan(
        &self,
        command: &StartRun,
        scope: &run_domain::RunCleanupHostId,
        vm: &vm_trait::VmId,
    ) -> Result<CreateRunResult, RepositoryError> {
        self.inner.create_run_with_vm_plan(command, scope, vm).await
    }
    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError> {
        self.inner.ensure_runtime_git_provenance(run).await
    }
    async fn get(&self, run: RunId) -> Result<Run, RepositoryError> {
        if self.queued.load(Ordering::SeqCst) && self.cleanup.state.lock().await.target.is_none() {
            return Err(RepositoryError::InvalidData(
                "queued strict hydration before preparation",
            ));
        }
        self.inner.get(run).await
    }
    async fn bind_resources(
        &self,
        run: RunId,
        volume: Option<VolumeId>,
        lease: Option<LeaseId>,
        fence: Option<i64>,
        vm: &str,
    ) -> Result<Run, RepositoryError> {
        self.inner
            .bind_resources(run, volume, lease, fence, vm)
            .await
    }
    async fn transition(
        &self,
        run: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        self.inner.transition(run, next, exit, failure).await
    }
    async fn append_vm_event(
        &self,
        run: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        self.inner.append_vm_event(run, event).await
    }
    async fn request_cancel(&self, command: &CancelRun) -> Result<bool, RepositoryError> {
        let inserted = self.controls.lock().await.insert(command.command_id);
        let cancelled = self.inner.run.lock().await.cancel_requested_at.is_some();
        if inserted && !cancelled {
            self.inner.request_cancel(command).await?;
        }
        Ok(inserted)
    }
    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        self.inner.recoverable_runs().await
    }
}
