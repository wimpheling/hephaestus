//! Admission and cancellation evidence for decorated provisioning only.

use std::sync::{Mutex, MutexGuard};

use vm_trait::{VmError, VmId};

use super::{ProvisionAttempt, ProvisionOutcome, ProvisioningSnapshot, WorkerObservation};

struct LedgerState {
    closed: bool,
    tracking_failed: bool,
    attempts: Vec<ProvisionAttempt>,
}

/// A single mutex makes admission, closure, and outcome observation atomic.
pub(super) struct AttemptLedger {
    state: Mutex<LedgerState>,
}

impl AttemptLedger {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(LedgerState {
                closed: false,
                tracking_failed: false,
                attempts: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, LedgerState>, VmError> {
        let state = self
            .state
            .lock()
            .map_err(|_| VmError::InvalidState("publication provision ledger is poisoned"))?;
        if state.tracking_failed {
            return Err(VmError::InvalidState(
                "publication provision tracking failed",
            ));
        }
        Ok(state)
    }

    pub(super) fn admit(&self, id: VmId) -> Result<AttemptGuard<'_>, VmError> {
        let mut state = self.lock()?;
        if state.closed {
            return Err(VmError::InvalidState("publication provisioning is closed"));
        }
        let index = state.attempts.len();
        state.attempts.push(ProvisionAttempt {
            id,
            outcome: ProvisionOutcome::InFlight,
            physical: None,
            observation_failed: false,
        });
        drop(state);
        Ok(AttemptGuard {
            ledger: self,
            index,
            returned: false,
        })
    }

    pub(super) fn close(&self) -> Result<ProvisioningSnapshot, VmError> {
        let mut state = self.lock()?;
        state.closed = true;
        let snapshot = Self::snapshot_locked(&state);
        drop(state);
        Ok(snapshot)
    }

    pub(super) fn snapshot(&self) -> Result<ProvisioningSnapshot, VmError> {
        let state = self.lock()?;
        Ok(Self::snapshot_locked(&state))
    }

    fn snapshot_locked(state: &LedgerState) -> ProvisioningSnapshot {
        ProvisioningSnapshot {
            closed: state.closed,
            attempts: state.attempts.clone(),
        }
    }

    fn record_outcome(
        &self,
        index: usize,
        outcome: ProvisionOutcome,
        observation: Option<Result<WorkerObservation, VmError>>,
    ) {
        let (physical, observation_failed) = match observation {
            Some(Ok(physical)) => (Some(physical), false),
            Some(Err(_)) => (None, true),
            None => (None, false),
        };
        // A poisoned lock remains permanently unavailable to admission/snapshots.
        // Preserve the actual provider result even if ledger accounting fails.
        if let Ok(mut state) = self.state.lock() {
            if let Some(attempt) = state.attempts.get_mut(index) {
                attempt.outcome = outcome;
                attempt.physical = physical;
                attempt.observation_failed = observation_failed;
                state.tracking_failed |= observation_failed;
            } else {
                state.tracking_failed = true;
            }
        }
    }
}

/// Lives across the actual provider await; dropping it retains uncertainty.
pub(super) struct AttemptGuard<'a> {
    ledger: &'a AttemptLedger,
    index: usize,
    returned: bool,
}

impl AttemptGuard<'_> {
    pub(super) fn returned(
        mut self,
        succeeded: bool,
        observation: Option<Result<WorkerObservation, VmError>>,
    ) {
        let outcome = if succeeded {
            ProvisionOutcome::ReturnedHandle
        } else {
            ProvisionOutcome::ReturnedError
        };
        self.ledger.record_outcome(self.index, outcome, observation);
        self.returned = true;
    }
}

impl Drop for AttemptGuard<'_> {
    fn drop(&mut self) {
        if !self.returned {
            self.ledger
                .record_outcome(self.index, ProvisionOutcome::CancelledOrUnknown, None);
        }
    }
}
