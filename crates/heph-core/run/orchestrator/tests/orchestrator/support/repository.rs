use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{LeaseId, RunId, VolumeId};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use vm_trait::VmExit;

pub struct MemoryRepository {
    pub run: Mutex<Run>,
    pub created: Mutex<bool>,
    pub events: Mutex<Vec<StoredVmEvent>>,
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
        }
    }
}

#[async_trait]
impl RunRepository for MemoryRepository {
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

    async fn ensure_runtime_git_provenance(&self, _run: &Run) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn get(&self, _run_id: RunId) -> Result<Run, RepositoryError> {
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
