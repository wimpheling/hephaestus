//! Original requests, outcomes and actual join handles remain node-owned.
use super::{Arc, Failure, ManagedRecipe, Outcome, Request};
use recipe_application::{CommandIdentity, DeploymentId};
use std::{collections::BTreeMap, sync::MutexGuard};
use tokio::{
    sync::{Mutex as AsyncMutex, watch},
    task::JoinHandle,
};

pub(super) type Handle = Arc<AsyncMutex<Option<JoinHandle<()>>>>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Running,
    Settled,
    Uncertain,
}
/// Causal comparison data only; entering an engine is not native admission.
#[derive(Clone, Copy)]
pub(super) struct Snapshot {
    pub phase: Phase,
    pub command: Option<CommandIdentity>,
}
pub(super) struct Tail {
    pub phase: watch::Receiver<Snapshot>,
}
pub(super) struct Entry {
    pub request: Arc<Request>,
    pub handle: Handle,
    pub outcome: Option<Outcome>,
    pub join_error: Option<Arc<Failure>>,
    pub joined: bool,
}
#[derive(Default)]
pub(super) struct State {
    pub closed: bool,
    pub poisoned: bool,
    pub next: u64,
    pub tails: BTreeMap<DeploymentId, Tail>,
    pub entries: BTreeMap<u64, Entry>,
}
impl ManagedRecipe {
    pub(super) fn state(&self) -> MutexGuard<'_, State> {
        self.jobs.lock().unwrap_or_else(|cause| {
            let mut state = cause.into_inner();
            state.poisoned = true;
            state.closed = true;
            state
        })
    }
    pub(crate) fn close(&self) {
        self.state().closed = true;
    }
    pub(super) fn finish(&self, sequence: u64, outcome: Outcome) {
        let mut state = self.state();
        if let Some(entry) = state.entries.get_mut(&sequence) {
            entry.outcome = Some(outcome);
        } else {
            state.closed = true;
            state.poisoned = true;
        }
    }

    /// Actual Stop hook: settle original jobs, not physical resources.
    ///
    /// Closing/provider detach must precede this wait when DB/planning is stalled.
    /// Caller cancellation leaves each borrowed actual handle in its owning slot.
    /// Final native/ledger observations remain separate from these actual joins.
    pub(crate) async fn join_jobs(&self) -> Result<(), Arc<Failure>> {
        let handles = {
            let mut state = self.state();
            state.closed = true;
            state
                .entries
                .iter()
                .map(|(id, entry)| (*id, entry.handle.clone()))
                .collect::<Vec<_>>()
        };
        for (sequence, handle) in handles {
            let mut slot = handle.lock().await;
            if let Some(task) = slot.as_mut() {
                let joined = task.await;
                // Cache the real join result before removing a consumed future.
                let mut state = self.state();
                let entry = state
                    .entries
                    .get_mut(&sequence)
                    .ok_or_else(|| Arc::new(Failure::Unsettled))?;
                entry.join_error = joined.err().map(|cause| Arc::new(Failure::Join(cause)));
                entry.joined = true;
                drop(state);
                drop(slot.take());
            }
            drop(slot);
        }
        let state = self.state();
        if state.poisoned
            || state
                .entries
                .values()
                .any(|entry| !entry.joined || entry.outcome.is_none())
        {
            return Err(Arc::new(Failure::Unsettled));
        }
        let failure = state.entries.values().find_map(|entry| {
            entry.join_error.clone().or_else(|| {
                entry
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.as_ref().err())
                    .filter(|cause| matches!(cause.as_ref(), Failure::Unsettled | Failure::Join(_)))
                    .cloned()
            })
        });
        for (sequence, entry) in &state.entries {
            if entry.outcome.as_ref().is_some_and(Result::is_err) {
                // The actual error remains in the entry; never echo request/plan values.
                tracing::debug!(
                    sequence,
                    "managed recipe original logical error retained after actual join"
                );
            }
        }
        drop(state);
        failure.map_or(Ok(()), Err)
    }
}
