use super::super::workspace::pin::WorkspacePin;
use super::preparation::Preparation;
use super::{
    BuildCleanupPhase, BuildCleanupStatus, BuildExecutionError, BuildRequestId, lock,
    models::{Mode, Outcome},
};
use crate::BuildInput;
use std::sync::{Arc, Mutex};
use vm_trait::{VmError, VmInstance, VmProvisionAttempt};

pub(super) enum Workspace {
    Untouched,
    Acquiring { _original_path: std::path::PathBuf },
    Original(Arc<WorkspacePin>),
}
pub(super) struct ClaimPins {
    pub(super) input: BuildInput,
    pub(super) release: release_domain::ReleaseId,
    pub(super) agent: release_domain::ReleaseAgentId,
    pub(super) version: release_domain::ReleaseVersion,
}
pub(super) struct State {
    pub(super) mode: Mode,
    pub(super) outcome: Option<Outcome>,
    pub(super) history: Vec<Outcome>,
    pub(super) workspace: Workspace,
    pub(super) preparation: Option<Arc<Preparation>>,
    pub(super) attempt: Option<Arc<dyn VmProvisionAttempt>>,
    pub(super) instance: Option<Arc<dyn VmInstance>>,
    pub(super) detached: bool,
    pub(super) closing: bool,
    pub(super) finalization: bool,
    pub(super) cleanup_failure: Option<Arc<VmError>>,
    pub(super) cleanup_error: Option<Arc<BuildExecutionError>>,
    pub(super) errors: Vec<Arc<dyn std::error::Error + Send + Sync>>,
    pub(super) claim: Option<ClaimPins>,
    pub(super) completion: Option<Arc<super::context::Completion>>,
    pub(super) git_children: Vec<Arc<super::super::workspace::git_child::GitChild>>,
}
pub(in super::super) struct Node {
    pub(super) id: BuildRequestId,
    pub(super) state: Mutex<State>,
    pub(super) changed: tokio::sync::Notify,
    pub(super) vm_operation: tokio::sync::Mutex<()>,
    pub(super) cleanup_job: Mutex<Option<Arc<super::task::Task<()>>>>,
    pub(super) git_cleanup: Mutex<Vec<Arc<super::git_cleanup::GitCleanup>>>,
    #[cfg(test)]
    pub(super) watcher_control: Mutex<Option<Arc<super::task::WatcherControl>>>,
    pub(super) work_job: Mutex<Option<Arc<super::task::Task<Outcome>>>>,
    pub(super) execution_job: Mutex<Option<Arc<super::task::Task<()>>>>,
}
impl Node {
    pub(in super::super) const fn id(&self) -> BuildRequestId {
        self.id
    }
    pub(super) fn new(id: BuildRequestId, mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            id,
            state: Mutex::new(State {
                mode,
                outcome: None,
                history: Vec::new(),
                workspace: Workspace::Untouched,
                preparation: None,
                attempt: None,
                instance: None,
                detached: true,
                closing: false,
                finalization: false,
                cleanup_failure: None,
                cleanup_error: None,
                errors: Vec::new(),
                claim: None,
                completion: None,
                git_children: Vec::new(),
            }),
            changed: tokio::sync::Notify::new(),
            vm_operation: tokio::sync::Mutex::new(()),
            cleanup_job: Mutex::new(None),
            execution_job: Mutex::new(None),
            work_job: Mutex::new(None),
            #[cfg(test)]
            watcher_control: Mutex::new(None),
            git_cleanup: Mutex::new(Vec::new()),
        })
    }
    pub(in super::super) fn check_open(&self) -> Result<(), BuildExecutionError> {
        if lock(&self.state).closing {
            Err(BuildExecutionError::Closed)
        } else {
            Ok(())
        }
    }
    pub(in super::super) fn close(&self) {
        lock(&self.state).closing = true;
        self.changed.notify_waiters();
    }
    pub(in super::super) fn begin_workspace(
        &self,
        path: std::path::PathBuf,
    ) -> Result<(), BuildExecutionError> {
        let mut state = lock(&self.state);
        if state.closing {
            return Err(BuildExecutionError::Closed);
        }
        if !matches!(state.workspace, Workspace::Untouched) {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
        state.workspace = Workspace::Acquiring {
            _original_path: path,
        };
        drop(state);
        Ok(())
    }
    pub(in super::super) fn git_child(
        &self,
        child: Arc<super::super::workspace::git_child::GitChild>,
    ) -> Result<(), BuildExecutionError> {
        let mut state = lock(&self.state);
        if state.closing {
            return Err(BuildExecutionError::Closed);
        }
        state.git_children.push(child);
        self.changed.notify_waiters();
        drop(state);
        Ok(())
    }
    pub(in super::super) fn workspace(&self, pin: Arc<WorkspacePin>) {
        lock(&self.state).workspace = Workspace::Original(pin);
        self.changed.notify_waiters();
    }
    pub(in super::super) fn original_workspace(
        &self,
    ) -> Result<Arc<WorkspacePin>, BuildExecutionError> {
        match &lock(&self.state).workspace {
            Workspace::Original(pin) => Ok(pin.clone()),
            _ => Err(BuildExecutionError::UnsafeWorkspace),
        }
    }
    pub(in super::super) fn install_attempt(
        &self,
        attempt: Arc<dyn VmProvisionAttempt>,
    ) -> Result<(), BuildExecutionError> {
        let mut state = lock(&self.state);
        if state.attempt.is_some() {
            return Err(BuildExecutionError::StoredState);
        }
        state.attempt = Some(attempt);
        state.detached = false;
        self.changed.notify_waiters();
        if state.closing {
            Err(BuildExecutionError::Closed)
        } else {
            Ok(())
        }
    }
    pub(in super::super) fn instance(&self, instance: Arc<dyn VmInstance>) {
        lock(&self.state).instance = Some(instance);
        self.changed.notify_waiters();
    }
    pub(in super::super) fn vm_error(&self, error: VmError) {
        lock(&self.state).errors.push(Arc::new(error));
    }
    pub(in super::super) fn error(&self, error: Arc<dyn std::error::Error + Send + Sync>) {
        lock(&self.state).errors.push(error);
    }
    pub(in super::super) async fn detach(&self) -> Result<(), BuildExecutionError> {
        let _operation = self.vm_operation.lock().await;
        let attempt = {
            let state = lock(&self.state);
            if state.detached {
                return Ok(());
            }
            state
                .attempt
                .clone()
                .ok_or(BuildExecutionError::VmCleanup)?
        };
        match attempt.cleanup().await {
            Ok(()) => {
                {
                    let mut state = lock(&self.state);
                    state.detached = true;
                    state.cleanup_failure = None;
                }
                self.changed.notify_waiters();
                Ok(())
            }
            Err(error) => {
                let error = Arc::new(error);
                let mut state = lock(&self.state);
                state.errors.push(error.clone());
                state.cleanup_failure = Some(error);
                drop(state);
                Err(BuildExecutionError::VmCleanup)
            }
        }
    }
    pub(in super::super) fn require_detached(&self) -> Result<(), BuildExecutionError> {
        if lock(&self.state).detached {
            Ok(())
        } else {
            Err(BuildExecutionError::VmCleanup)
        }
    }
    pub(in super::super) fn preserve_finalization(&self) {
        lock(&self.state).finalization = true;
    }
    pub(in super::super) fn finalized(&self) {
        lock(&self.state).finalization = false;
    }
    pub(in super::super) fn remove_workspace(&self) -> Result<(), BuildExecutionError> {
        self.require_detached()?;
        if lock(&self.state).finalization {
            return Err(BuildExecutionError::FinalizationRequired);
        }
        let workspace = {
            let mut state = lock(&self.state);
            match &state.workspace {
                Workspace::Untouched => {
                    if state
                        .completion
                        .as_ref()
                        .is_some_and(|completion| completion.clears_finalized_hold(&state))
                    {
                        state.cleanup_error = None;
                    }
                    drop(state);
                    return Ok(());
                }
                Workspace::Acquiring { .. } => return Err(BuildExecutionError::UnsafeWorkspace),
                Workspace::Original(pin) => pin.clone(),
            }
        };
        workspace.remove()?;
        lock(&self.state).cleanup_error = None;
        Ok(())
    }
    pub(in super::super) fn settled(&self) -> bool {
        self.status().phase == BuildCleanupPhase::Settled
    }
    pub(in super::super) fn status(&self) -> BuildCleanupStatus {
        let jobs_settled = self.jobs_settled();
        let state = lock(&self.state);
        let phase = if state.outcome.is_none() || !jobs_settled {
            BuildCleanupPhase::Executing
        } else if !state.detached {
            BuildCleanupPhase::VmHeld
        } else if state.finalization {
            BuildCleanupPhase::FinalizationRequired
        } else if state.cleanup_error.is_some()
            || match &state.workspace {
                Workspace::Untouched => false,
                Workspace::Acquiring { .. } => true,
                Workspace::Original(pin) => !pin.removed(),
            }
        {
            BuildCleanupPhase::WorkspaceHeld
        } else {
            BuildCleanupPhase::Settled
        };
        BuildCleanupStatus {
            build_request_id: self.id,
            phase,
            retained_errors: state.errors.len(),
        }
    }
    pub(super) async fn outcome(&self) -> Outcome {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let outcome = { lock(&self.state).outcome.clone() };
            if let Some(outcome) = outcome {
                return outcome;
            }
            changed.await;
        }
    }
}
