use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use run_domain::{CancelRun, RunKind, StartRun};
use run_orchestrator::{OrchestratorError, RunCommandExecutor};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use tokio::sync::Semaphore;

pub enum Mode {
    Success,
    FailFirst,
    Blocked,
}

pub struct ControlledExecutor {
    mode: Mode,
    attempts: AtomicUsize,
    starts: Mutex<Vec<StartRun>>,
    cancels: Mutex<Vec<CancelRun>>,
    entered: Semaphore,
    released: Semaphore,
    cancelled: Semaphore,
}

impl ControlledExecutor {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            attempts: AtomicUsize::new(0),
            starts: Mutex::new(Vec::new()),
            cancels: Mutex::new(Vec::new()),
            entered: Semaphore::new(0),
            released: Semaphore::new(0),
            cancelled: Semaphore::new(0),
        }
    }
    pub async fn wait_entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.entered.acquire())
            .await
            .expect("executor entered")
            .unwrap()
            .forget();
    }
    pub async fn wait_cancelled(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.cancelled.acquire())
            .await
            .expect("cancel executes while Start is still blocked")
            .unwrap()
            .forget();
    }
    pub fn release(&self) {
        self.released.add_permits(1);
    }
    pub fn starts(&self) -> Vec<StartRun> {
        self.starts.lock().unwrap().clone()
    }
    pub fn cancels(&self) -> Vec<CancelRun> {
        self.cancels.lock().unwrap().clone()
    }
}

#[async_trait]
impl RunCommandExecutor for ControlledExecutor {
    async fn start_run(&self, command: &StartRun) -> Result<(), OrchestratorError> {
        self.starts.lock().unwrap().push(command.clone());
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        self.entered.add_permits(1);
        match self.mode {
            Mode::FailFirst if attempt == 0 => {
                return Err(OrchestratorError::RunInProgress(command.run_id));
            }
            Mode::Blocked => self.released.acquire().await.unwrap().forget(),
            Mode::Success | Mode::FailFirst => {}
        }
        Ok(())
    }
    async fn cancel_run(&self, command: &CancelRun) -> Result<bool, OrchestratorError> {
        self.cancels.lock().unwrap().push(command.clone());
        self.cancelled.add_permits(1);
        Ok(true)
    }
}

pub fn command() -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    }
}
