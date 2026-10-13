//! Qualified bootstrap ordering and the private actual-exit failure boundary.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
use tokio::sync::Notify;
use vm_trait::VmInstance;

use super::{OrchestratorError, RunOrchestrator};

const EXECUTING: u8 = 0;
const AUTHORITY_FAILED: u8 = 1;
const EXITED: u8 = 2;

#[derive(Default)]
pub(super) struct InvocationExecutionPhase {
    state: AtomicU8,
    failure: Mutex<Option<OrchestratorError>>,
    failed: Notify,
    exited: Notify,
}

impl InvocationExecutionPhase {
    // Only the actual successful wait-and-event-drain path may call this.
    pub fn complete_wait(&self) -> bool {
        let exited = self
            .state
            .compare_exchange(EXECUTING, EXITED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if exited {
            self.exited.notify_one();
        }
        exited
    }

    pub fn exited(&self) -> bool {
        self.state.load(Ordering::SeqCst) == EXITED
    }

    pub async fn exit_signal(&self) {
        loop {
            let notified = self.exited.notified();
            if self.exited() {
                return;
            }
            notified.await;
        }
    }

    pub fn fail(&self, error: OrchestratorError) {
        // No await occurs between winning the CAS and publishing the original
        // owned error. Recovering this private slot after unwind grants no IO.
        let mut failure = self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let published = self
            .state
            .compare_exchange(
                EXECUTING,
                AUTHORITY_FAILED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok();
        if published {
            *failure = Some(error);
        }
        drop(failure);
        if published {
            self.failed.notify_one();
        }
    }

    pub fn take_failure(&self) -> Option<OrchestratorError> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub async fn failure_signal(&self) {
        loop {
            let notified = self.failed.notified();
            if self.state.load(Ordering::SeqCst) == AUTHORITY_FAILED {
                return;
            }
            notified.await;
        }
    }
}

// The stored Provisioning interval includes actual Start, exact guest ACK and
// complete physical attachment. Frozen B1 predicates guard both later states.
pub(super) async fn enter_running(
    orchestrator: &RunOrchestrator,
    run: &run_domain::Run,
    instance: &Arc<dyn VmInstance>,
) -> Result<run_domain::Run, OrchestratorError> {
    let progress = async {
        orchestrator
            .repository
            .transition(run.id, run_domain::RunState::Starting, None, None)
            .await?;
        orchestrator
            .repository
            .transition(run.id, run_domain::RunState::Running, None, None)
            .await
    }
    .await;
    match progress {
        Ok(run) => Ok(run),
        Err(error) => {
            orchestrator.abort_vm_keep_lease(run.id, instance).await;
            Err(error.into())
        }
    }
}

#[cfg(test)]
#[path = "invocation_lifecycle/tests.rs"]
mod tests;
