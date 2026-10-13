//! Existing diagnostic-only original generation inventory.
use super::{Custody, Registry, VmAttemptInventory, VmAttemptPhase};
use crate::provider::{
    common::{Arc, Lifecycle, Ordering},
    launch::LaunchState,
};

impl Registry {
    pub(in crate::provider) async fn inventory(&self) -> VmAttemptInventory {
        let (closed, records) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let records = state
                .records
                .values()
                .map(|record| {
                    let custody = match &record.custody {
                        Custody::Pending(entry) => Custody::Pending(Arc::clone(entry)),
                        Custody::Instance(instance) => Custody::Instance(Arc::clone(instance)),
                        Custody::Cleaned => Custody::Cleaned,
                    };
                    (record.snapshot.clone(), custody, record.launch_active)
                })
                .collect::<Vec<_>>();
            (state.closed, records)
        };
        let mut attempts = Vec::with_capacity(records.len());
        for (mut snapshot, custody, launch_active) in records {
            match custody {
                Custody::Pending(entry) => {
                    match entry.custody.process() {
                        Ok(process) => {
                            snapshot.worker_pid = process.and_then(|process| process.pid);
                        }
                        Err(error) => snapshot.error = Some(error.to_string()),
                    }
                    let launch_state = entry.state.borrow().clone();
                    match launch_state {
                        LaunchState::Held(error) => {
                            snapshot.phase = VmAttemptPhase::CleanupPending;
                            snapshot.error = Some(error.to_string());
                        }
                        LaunchState::Cleaned => snapshot.phase = VmAttemptPhase::Cleaned,
                        LaunchState::Handoff => snapshot.phase = VmAttemptPhase::Handoff,
                        LaunchState::Working | LaunchState::HandedOff => {}
                    }
                }
                Custody::Instance(instance) => {
                    snapshot.phase = if instance.cleanup_complete.load(Ordering::Acquire) {
                        VmAttemptPhase::Cleaned
                    } else {
                        match &*instance.state.lock().await {
                            Lifecycle::Provisioned if launch_active => VmAttemptPhase::Handoff,
                            Lifecycle::Provisioned => VmAttemptPhase::Provisioned,
                            Lifecycle::Starting => VmAttemptPhase::Starting,
                            Lifecycle::Running => VmAttemptPhase::Running,
                            Lifecycle::Stopping => VmAttemptPhase::Stopping,
                            Lifecycle::Exited => VmAttemptPhase::ExitObserved,
                            Lifecycle::StartFailed(_) => VmAttemptPhase::StartFailed,
                            Lifecycle::Destroying { .. } | Lifecycle::Destroyed => {
                                VmAttemptPhase::CleanupPending
                            }
                        }
                    };
                }
                Custody::Cleaned => {}
            }
            attempts.push(snapshot);
        }
        VmAttemptInventory {
            admission_closed: closed,
            attempts,
        }
    }
}
