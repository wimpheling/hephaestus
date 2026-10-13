//! Controlled guest and panic-on-use Git ports; no native VM or SQL proof.
use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tokio::sync::{broadcast, watch};
use vm_trait::{
    StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec,
};
use workspace_domain::{
    PreparedRuntimeGitWorkspace, PreparedWorkspace, PublishedResult, RunWorkspaceManager,
    RuntimeGitWorkspaceManager, WorkspaceError,
};

use crate::canonical::provider::ScopedProvider;
use crate::support::lock;

pub struct NoGit;
#[async_trait]
impl RunWorkspaceManager for NoGit {
    async fn prepare(&self, _: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        panic!("Invocation workspace preparation");
    }
    async fn finalize(&self, _: &Run, _: &str) -> Result<Option<PublishedResult>, WorkspaceError> {
        panic!("Invocation workspace finalize");
    }
    async fn abandon(&self, _: RunId) -> Result<(), WorkspaceError> {
        panic!("Invocation workspace abandon");
    }
    async fn recover(&self) -> Result<usize, WorkspaceError> {
        panic!("Invocation workspace recovery");
    }
}
#[async_trait]
impl RuntimeGitWorkspaceManager for NoGit {
    async fn prepare_runtime_git(
        &self,
        _: &Run,
    ) -> Result<Option<PreparedRuntimeGitWorkspace>, WorkspaceError> {
        panic!("Invocation runtime Git preparation");
    }
    async fn abandon_runtime_git(&self, _: RunId) -> Result<(), WorkspaceError> {
        panic!("Invocation runtime Git abandon");
    }
    async fn recover_runtime_git(&self) -> Result<usize, WorkspaceError> {
        panic!("Invocation runtime Git recovery");
    }
}

pub struct ControlledProvider {
    pub owner: Arc<ScopedProvider>,
    pub exit: watch::Sender<Option<VmExit>>,
    pub spec: Mutex<Option<VmSpec>>,
    pub events: broadcast::Sender<VmEvent>,
    pub stop_exits: Arc<AtomicBool>,
    pub acknowledgement: Arc<AtomicU8>,
    pub fail_wait: Arc<AtomicBool>,
    pub emit_exit_event: Arc<AtomicBool>,
}
impl ControlledProvider {
    pub fn new(owner: Arc<ScopedProvider>) -> Self {
        Self {
            owner,
            acknowledgement: Arc::new(AtomicU8::new(0)),
            fail_wait: Arc::new(AtomicBool::new(false)),
            emit_exit_event: Arc::new(AtomicBool::new(false)),
            stop_exits: Arc::new(AtomicBool::new(true)),
            exit: watch::channel(None).0,
            spec: Mutex::new(None),
            events: broadcast::channel(32).0,
        }
    }
    pub fn finish(&self, code: i32) {
        self.exit.send_replace(Some(VmExit {
            code: Some(code),
            signal: None,
        }));
    }
}
#[async_trait]
impl VmProvider for ControlledProvider {
    fn name(&self) -> &'static str {
        "controlled-invocation"
    }
    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        self.owner.owner_scope()
    }
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        lock(&self.owner.inner.log).push("provision");
        let id = spec.id.clone();
        let authority = spec
            .runtime_authority
            .as_ref()
            .map(|bootstrap| (bootstrap.session_id(), bootstrap.generation()));
        *lock(&self.spec) = Some(spec);
        Ok(Arc::new(ControlledGuest {
            id,
            authority,
            acknowledgement: self.acknowledgement.clone(),
            fail_wait: self.fail_wait.clone(),
            emit_exit_event: self.emit_exit_event.clone(),
            pause_start: lock(&self.owner.pause_start).clone(),
            events: self.events.clone(),
            exit: self.exit.clone(),
            log: self.owner.inner.log.clone(),
            stop_exits: self.stop_exits.clone(),
        }))
    }
    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        panic!("Invocation unscoped cleanup");
    }
    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        vm: &VmId,
    ) -> Result<(), VmError> {
        self.owner.cleanup_orphan_scoped(scope, vm).await
    }
}
struct ControlledGuest {
    id: VmId,
    authority: Option<(uuid::Uuid, u64)>,
    acknowledgement: Arc<AtomicU8>,
    fail_wait: Arc<AtomicBool>,
    emit_exit_event: Arc<AtomicBool>,
    pause_start: Option<Arc<crate::canonical::provider::Pause>>,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
    log: Arc<Mutex<Vec<&'static str>>>,
    stop_exits: Arc<AtomicBool>,
}
#[async_trait]
impl VmInstance for ControlledGuest {
    fn id(&self) -> &VmId {
        &self.id
    }
    async fn start(&self) -> Result<(), VmError> {
        lock(&self.log).push("start");
        if let Some(pause) = &self.pause_start {
            pause.wait().await;
        }
        if let Some((session_id, generation)) = self.authority
            && self.acknowledgement.load(Ordering::SeqCst) != 2
        {
            let generation =
                generation + u64::from(self.acknowledgement.load(Ordering::SeqCst) == 1);
            let _ = self.events.send(VmEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            });
        }
        let _ = self.events.send(VmEvent::Ready);
        let _ = self.events.send(VmEvent::FinalizeResult {
            message: "untrusted Git finalize".into(),
        });
        Ok(())
    }
    async fn stop(&self, _: StopMode) -> Result<(), VmError> {
        lock(&self.log).push("stop");
        if self.stop_exits.load(Ordering::SeqCst) {
            self.exit.send_replace(Some(VmExit {
                code: None,
                signal: Some(15),
            }));
        }
        Ok(())
    }
    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let exit = receiver.borrow_and_update().clone();
            if let Some(exit) = exit {
                if self.fail_wait.load(Ordering::SeqCst) {
                    return Err(VmError::InvalidState("controlled actual wait failed"));
                }
                if self.emit_exit_event.load(Ordering::SeqCst) {
                    let _ = self.events.send(VmEvent::Exited(exit.clone()));
                }
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("controlled exit channel closed"))?;
        }
    }
    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }
    async fn destroy(&self) -> Result<(), VmError> {
        lock(&self.log).push("destroy");
        Ok(())
    }
}

pub struct GitOverrideFactory;
#[async_trait]
impl run_orchestrator::VmSpecFactory for GitOverrideFactory {
    async fn build(&self, _: &Run) -> Result<VmSpec, VmError> {
        panic!("Invocation scalar factory");
    }
    async fn build_with_volumes(
        &self,
        run: &Run,
        _: &volume_domain::RunVolumeSelections,
    ) -> Result<VmSpec, VmError> {
        let mut spec =
            run_orchestrator::VmSpecFactory::build(&crate::support::TestSpecFactory, run).await?;
        spec.command.working_dir = Some("/foreign/git-workspace".into());
        Ok(spec)
    }
}
