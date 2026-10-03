use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{LeaseId, RunId, VolumeId};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use vm_trait::VmExit;

#[derive(Clone)]
pub struct TransitionPause {
    pub entered: Arc<tokio::sync::Notify>,
    pub release: Arc<tokio::sync::Notify>,
}

pub struct MemoryRepository {
    pub run: Mutex<Run>,
    pub created: Mutex<bool>,
    pub events: Mutex<Vec<StoredVmEvent>>,
    pub pause_running_transition: Mutex<Option<TransitionPause>>,
    pub hang_events: AtomicBool,
    pub hang_get: AtomicBool,
    pub event_entered: tokio::sync::Notify,
    /// Controlled test evidence only; no real SQL admission is claimed.
    pub invocation_admitted: AtomicBool,
    pub planned_vm: Mutex<Option<(run_domain::RunCleanupHostId, vm_trait::VmId)>>,
}

impl MemoryRepository {
    pub fn new(command: &StartRun) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            run: Mutex::new(Run {
                id: command.run_id,
                instance_id: command.instance_id,
                instance_revision_id: command.instance_revision_id,
                release_id: command.release_id,
                release_agent_id: command.release_agent_id,
                attachment_id: command.attachment_id,
                kind: command.kind,
                command_id: command.command_id,
                requires_state: command.requires_state,
                volume_id: None,
                lease_id: None,
                lease_fencing_token: None,
                vm_id: None,
                state: RunState::Queued,
                outcome: None,
                exit: None,
                failure: None,
                cancel_requested_at: None,
                created_at: now,
                updated_at: now,
                state_version: 0,
            }),
            created: Mutex::new(false),
            events: Mutex::new(Vec::new()),
            pause_running_transition: Mutex::new(None),
            planned_vm: Mutex::new(None),
            invocation_admitted: AtomicBool::new(false),
            hang_events: AtomicBool::new(false),
            hang_get: AtomicBool::new(false),
            event_entered: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl RunRepository for MemoryRepository {
    async fn create_run_with_vm_plan(
        &self,
        command: &StartRun,
        scope: &run_domain::RunCleanupHostId,
        vm_id: &vm_trait::VmId,
    ) -> Result<CreateRunResult, RepositoryError> {
        let mut created = self.created.lock().await;
        let mut plan = self.planned_vm.lock().await;
        let mut run = self.run.lock().await;
        let same_run = run.id == command.run_id;
        if !same_run
            || run.command_id != command.command_id
            || run.instance_id != command.instance_id
            || run.instance_revision_id != command.instance_revision_id
            || run.release_id != command.release_id
            || run.release_agent_id != command.release_agent_id
            || run.requires_state != command.requires_state
            || run.kind != command.kind
            || run.attachment_id != command.attachment_id
        {
            return Err(RepositoryError::InvalidData("immutable run input differs"));
        }
        if command.kind == run_domain::RunKind::Invocation {
            if !self.invocation_admitted.load(Ordering::SeqCst)
                || !*created
                || plan.as_ref() != Some(&(scope.clone(), vm_id.clone()))
                || run.vm_id.as_deref() != Some(vm_id.0.as_str())
            {
                return Err(RepositoryError::InvalidData(
                    "protected Invocation admission is absent or differs",
                ));
            }
            return Ok(CreateRunResult {
                run: run.clone(),
                created: false,
            });
        }
        let was_created = !*created;
        if let Some(existing) = plan.as_ref() {
            if existing != &(scope.clone(), vm_id.clone()) {
                return Err(RepositoryError::InvalidData("immutable VM plan differs"));
            }
        } else if run.state != RunState::CleanedUp {
            if vm_id.0 != command.run_id.to_string() {
                return Err(RepositoryError::InvalidData("fresh VM identity differs"));
            }
            if *created || run.vm_id.is_some() {
                return Err(RepositoryError::InvalidData(
                    "historical run has no admission",
                ));
            }
            *plan = Some((scope.clone(), vm_id.clone()));
            run.vm_id = Some(vm_id.0.clone());
        }
        *created = true;
        let result = CreateRunResult {
            run: run.clone(),
            created: was_created,
        };
        drop(run);
        drop(plan);
        drop(created);
        Ok(result)
    }

    async fn create_run(&self, _command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        let mut created = self.created.lock().await;
        let was_created = !*created;
        *created = true;
        drop(created);
        Ok(CreateRunResult {
            run: self.run.lock().await.clone(),
            created: was_created,
        })
    }

    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError> {
        assert_ne!(
            run.kind,
            run_domain::RunKind::Invocation,
            "Invocation called Git provenance"
        );
        Ok(())
    }

    async fn get(&self, _run_id: RunId) -> Result<Run, RepositoryError> {
        if self.hang_get.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        Ok(self.run.lock().await.clone())
    }

    async fn bind_resources(
        &self,
        _run_id: RunId,
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        lease_fencing_token: Option<i64>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        let mut run = self.run.lock().await;
        run.volume_id = volume_id;
        run.lease_id = lease_id;
        run.lease_fencing_token = lease_fencing_token;
        run.vm_id = Some(vm_id.to_owned());
        Ok(run.clone())
    }

    async fn transition(
        &self,
        _run_id: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        if next == RunState::Running {
            let pause = self.pause_running_transition.lock().await.clone();
            if let Some(pause) = pause {
                pause.entered.notify_one();
                pause.release.notified().await;
            }
        }
        let mut run = self.run.lock().await;
        if !run.state.can_transition_to(next) {
            return Err(RepositoryError::InvalidTransition(
                run_domain::InvalidTransition {
                    current: run.state,
                    requested: next,
                },
            ));
        }
        run.state = next;
        run.outcome = next.outcome().or(run.outcome);
        run.exit = exit.cloned().or_else(|| run.exit.clone());
        run.failure = failure.map(str::to_owned).or_else(|| run.failure.clone());
        run.state_version += 1;
        run.updated_at = OffsetDateTime::now_utc();
        Ok(run.clone())
    }

    async fn append_vm_event(
        &self,
        _run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        self.event_entered.notify_one();
        if self.hang_events.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        self.events.lock().await.push(event);
        Ok(())
    }

    async fn request_cancel(&self, _command: &CancelRun) -> Result<bool, RepositoryError> {
        self.run.lock().await.cancel_requested_at = Some(OffsetDateTime::now_utc());
        Ok(true)
    }

    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        Ok(vec![self.run.lock().await.clone()])
    }
}
