//! One original clone family's synchronous admission and owning-task registry.
#[cfg(test)]
mod tests;

use super::held;
use heph_run::{CancelRun, StartRun};
use run_orchestrator::{OrchestratorError, RunCleanupWakeRequest, SupervisorReconciliationCause};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::{sync::Notify, task::JoinHandle};

#[derive(Clone)]
pub(super) enum Request {
    Start(StartRun),
    Cancel(CancelRun),
    Wake(RunCleanupWakeRequest),
    Reconcile(SupervisorReconciliationCause),
}
impl Request {
    const fn is_lifecycle(&self) -> bool {
        matches!(self, Self::Reconcile(_))
    }
    pub(super) const fn run_id(&self) -> Option<runtime_types::RunId> {
        match self {
            Self::Start(command) => Some(command.run_id),
            Self::Cancel(command) => Some(command.run_id),
            Self::Wake(request) => Some(request.run_id()),
            Self::Reconcile(_) => None,
        }
    }
}

enum Outcome {
    Running,
    Finished,
    Failed(Arc<OrchestratorError>),
    Uncertain(Arc<OrchestratorError>),
}
struct Entry {
    request: Request,
    // No handle is aborted/dropped as evidence. Completion remains independent.
    handle: Option<JoinHandle<()>>,
    outcome: Outcome,
    joined: bool,
}
#[derive(Default)]
struct State {
    closed: bool,
    paused: bool,
    next: u64,
    entries: BTreeMap<u64, Entry>,
}
struct OriginalControllers {
    _invocation: Arc<crate::ManagedInvocationController>,
    _legacy: Arc<crate::ManagedLegacyController>,
}

pub(super) struct Operations {
    // No controller has an operations back-reference. Even settled/held records
    // remain backed by the service's original roots, pools and guard registry.
    _owners: Option<OriginalControllers>,
    state: Mutex<State>,
    changed: Notify,
}
impl Operations {
    pub(super) fn new(
        invocation: Arc<crate::ManagedInvocationController>,
        legacy: Arc<crate::ManagedLegacyController>,
    ) -> Self {
        Self {
            _owners: Some(OriginalControllers {
                _invocation: invocation,
                _legacy: legacy,
            }),
            state: Mutex::new(State::default()),
            changed: Notify::new(),
        }
    }
    fn state(&self) -> Result<MutexGuard<'_, State>, OrchestratorError> {
        self.state
            .lock()
            .map_err(|_| held("managed operation registry poisoned"))
    }
    #[cfg(test)]
    pub(super) fn take_test_task(&self, sequence: u64) -> JoinHandle<()> {
        let mut state = self.state().unwrap();
        let entry = state.entries.get_mut(&sequence).unwrap();
        assert!(matches!(entry.outcome, Outcome::Running));
        let handle = entry.handle.take().unwrap();
        drop(state);
        handle
    }
    pub(super) fn close(&self) -> Result<(), OrchestratorError> {
        self.state()?.closed = true;
        self.changed.notify_waiters();
        Ok(())
    }
    pub(super) fn register(&self, request: Request) -> Result<u64, OrchestratorError> {
        let mut state = self.state()?;
        if matches!(request, Request::Start(_)) && (state.closed || state.paused) {
            return Err(held(
                "managed Start intake is closed or awaiting qualified reconciliation",
            ));
        }
        if request.is_lifecycle()
            && state.entries.values().any(|entry| {
                entry.request.is_lifecycle() && matches!(entry.outcome, Outcome::Running)
            })
        {
            return Err(held("managed lifecycle reconciliation is already owned"));
        }
        let sequence = state
            .next
            .checked_add(1)
            .ok_or_else(|| held("managed operation sequence exhausted"))?;
        if let Request::Reconcile(cause) = &request {
            state.paused = true;
            if *cause == SupervisorReconciliationCause::Stop {
                state.closed = true;
            }
        }
        state.next = sequence;
        state.entries.insert(
            sequence,
            Entry {
                request,
                handle: None,
                outcome: Outcome::Running,
                joined: false,
            },
        );
        drop(state);
        self.changed.notify_waiters();
        Ok(sequence)
    }
    pub(super) fn attach(
        &self,
        sequence: u64,
        handle: JoinHandle<()>,
    ) -> Result<(), OrchestratorError> {
        let mut state = self.state()?;
        let entry = state
            .entries
            .get_mut(&sequence)
            .ok_or_else(|| held("managed admitted operation missing"))?;
        if entry.handle.is_some() {
            return Err(held("managed operation already has an original task"));
        }
        entry.handle = Some(handle);
        drop(state);
        Ok(())
    }
    pub(super) fn finish(
        &self,
        sequence: u64,
        mut failure: Option<Arc<OrchestratorError>>,
    ) -> Result<Option<Arc<OrchestratorError>>, OrchestratorError> {
        let mut state = self.state()?;
        let lifecycle = state
            .entries
            .get(&sequence)
            .ok_or_else(|| held("managed operation completion missing"))?
            .request
            .is_lifecycle();
        if lifecycle
            && failure.is_none()
            && state.entries.iter().any(|(id, entry)| {
                *id != sequence
                    && (matches!(entry.outcome, Outcome::Uncertain(_))
                        || (!entry.request.is_lifecycle()
                            && (matches!(entry.outcome, Outcome::Running) || !entry.joined)))
            })
        {
            failure = Some(Arc::new(held(
                "managed cleanup submission raced lifecycle settlement",
            )));
        }
        let entry = state
            .entries
            .get_mut(&sequence)
            .ok_or_else(|| held("managed operation completion missing"))?;
        entry.outcome = failure.as_ref().map_or(Outcome::Finished, |cause| {
            Outcome::Failed(Arc::clone(cause))
        });
        if lifecycle && failure.is_none() {
            // This changes only pause: permanent close always wins under this lock.
            state.paused = false;
        }
        drop(state);
        self.changed.notify_waiters();
        Ok(failure)
    }
    pub(super) fn unexpected(&self, sequence: u64) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(entry) = state.entries.get_mut(&sequence) {
                entry.outcome = Outcome::Uncertain(Arc::new(held(
                    "managed original task ended without a qualified outcome",
                )));
                state.closed = true;
            }
        }
        self.changed.notify_waiters();
    }
    pub(super) fn idle_commands(&self) -> Result<bool, OrchestratorError> {
        let state = self.state()?;
        for entry in state.entries.values() {
            match &entry.outcome {
                Outcome::Uncertain(cause) => {
                    return Err(super::ownership::shared_error(Arc::clone(cause)));
                }
                Outcome::Running if !entry.request.is_lifecycle() => return Ok(false),
                _ => {}
            }
        }
        drop(state);
        Ok(true)
    }
    pub(super) fn failures(&self) -> Result<Vec<Arc<OrchestratorError>>, OrchestratorError> {
        Ok(self
            .state()?
            .entries
            .values()
            .filter_map(|entry| match &entry.outcome {
                Outcome::Failed(cause) => Some(Arc::clone(cause)),
                _ => None,
            })
            .collect())
    }
    fn settled_handles(&self) -> Result<Vec<(u64, JoinHandle<()>)>, OrchestratorError> {
        Ok(self
            .state()?
            .entries
            .iter_mut()
            .filter_map(|(sequence, entry)| {
                if entry.request.is_lifecycle() || matches!(entry.outcome, Outcome::Running) {
                    None
                } else {
                    entry.handle.take().map(|handle| (*sequence, handle))
                }
            })
            .collect())
    }
    fn join_failed(
        &self,
        sequence: u64,
        cause: tokio::task::JoinError,
    ) -> Result<(), OrchestratorError> {
        let original = Arc::new(run_orchestrator::RepositoryError::Storage(Box::new(cause)).into());
        let mut state = self.state()?;
        let entry = state
            .entries
            .get_mut(&sequence)
            .ok_or_else(|| held("managed original task association missing"))?;
        entry.outcome = Outcome::Uncertain(original);
        state.closed = true;
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }
    fn mark_joined(&self, sequence: u64) -> Result<(), OrchestratorError> {
        let mut state = self.state()?;
        state
            .entries
            .get_mut(&sequence)
            .ok_or_else(|| held("managed joined task association missing"))?
            .joined = true;
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }
    pub(super) fn joined_commands(&self) -> Result<bool, OrchestratorError> {
        let state = self.state()?;
        for entry in state.entries.values() {
            if let Outcome::Uncertain(cause) = &entry.outcome {
                return Err(super::ownership::shared_error(Arc::clone(cause)));
            }
            if !entry.request.is_lifecycle()
                && (matches!(entry.outcome, Outcome::Running) || !entry.joined)
            {
                return Ok(false);
            }
        }
        drop(state);
        Ok(true)
    }
    fn commands_joinable(&self) -> Result<bool, OrchestratorError> {
        Ok(self.state()?.entries.values().any(|entry| {
            !entry.request.is_lifecycle()
                && !matches!(entry.outcome, Outcome::Running)
                && entry.handle.is_some()
        }))
    }
    pub(super) async fn wait_commands(&self) -> Result<(), OrchestratorError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            for (sequence, handle) in self.settled_handles()? {
                if let Err(cause) = handle.await {
                    self.join_failed(sequence, cause)?;
                }
                self.mark_joined(sequence)?;
            }
            if self.joined_commands()? {
                return Ok(());
            }
            // A finished handle may have appeared after the scan. Re-scan it
            // without waiting on a notification that already happened.
            if self.commands_joinable()? {
                continue;
            }
            changed.await;
        }
    }
}

// Controlled protocol fixtures do not claim actual physical/backend custody.
#[cfg(test)]
impl Default for Operations {
    fn default() -> Self {
        Self {
            _owners: None,
            state: Mutex::new(State::default()),
            changed: Notify::new(),
        }
    }
}
