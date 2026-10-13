//! One original node slot, retaining real errors independently of result waiters.
use super::status::{ManagedHeldReason as Held, ManagedSupervisorPhase as Phase};
use super::{Control, ManagedServiceFailure, ManagedSupervisorStatus, NodeLease};
use crate::{AppError, managed_backend::custody::ManagedBackendNode};
use std::{
    error::Error,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::watch;

pub(super) struct Registry {
    state: Mutex<State>,
    pub(super) control: Arc<Control>,
    changed: watch::Sender<ManagedSupervisorStatus>,
}
// Registration, job settlement, acquisition uncertainty and poisoning are independent.
#[allow(clippy::struct_excessive_bools)]
struct State {
    status: ManagedSupervisorStatus,
    registered: bool,
    preparation_done: bool,
    acquisition_unknown: bool,
    cleanup: CleanupSchedule,
    poisoned: bool,
    inputs: Option<Arc<super::jobs::PreparationInputs>>,
    node: Option<Arc<ManagedBackendNode>>,
    failures: Vec<ManagedServiceFailure>,
}

pub(super) enum Action {
    Wait,
    Drain(Arc<ManagedBackendNode>, usize),
    Stop,
}

impl Registry {
    pub(super) fn new() -> (Arc<Self>, watch::Receiver<ManagedSupervisorStatus>) {
        let (changed, receiver) = watch::channel(ManagedSupervisorStatus::default());
        (
            Arc::new(Self {
                state: Mutex::new(State {
                    status: ManagedSupervisorStatus::default(),
                    registered: false,
                    preparation_done: false,
                    acquisition_unknown: false,
                    cleanup: CleanupSchedule::default(),
                    poisoned: false,
                    inputs: None,
                    node: None,
                    failures: Vec::new(),
                }),
                control: Arc::new(Control::default()),
                changed,
            }),
            receiver,
        )
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| {
            let mut state = error.into_inner();
            state.poisoned = true;
            state.status.phase = Phase::Held;
            state.status.held_reason = Some(Held::TaskFailure);
            state
        })
    }
    fn publish(&self, state: &State) {
        self.changed.send_replace(state.status.clone());
        self.control.changed.notify_one();
    }
    pub(super) fn register(
        &self,
        inputs: Arc<super::jobs::PreparationInputs>,
        status: watch::Receiver<ManagedSupervisorStatus>,
    ) -> Result<NodeLease, AppError> {
        let mut state = self.state();
        if state.poisoned || state.registered || self.control.is_closing() {
            return Err(AppError::Configuration(
                "managed service already registered, closing or uncertain".into(),
            ));
        }
        state.inputs = Some(inputs);
        state.registered = true;
        state.status.phase = Phase::Preparing;
        state.status.in_flight = true;
        self.publish(&state);
        drop(state);
        Ok(NodeLease {
            control: Arc::clone(&self.control),
            status,
        })
    }
    pub(super) fn install(&self, node: Arc<ManagedBackendNode>) {
        let mut state = self.state();
        state.node = Some(node);
        if !state.poisoned {
            state.status.phase = if self.control.is_closing() {
                Phase::Closing
            } else {
                Phase::Constructing
            };
        }
        self.publish(&state);
    }
    pub(super) fn prepared(&self) {
        let mut state = self.state();
        state.preparation_done = true;
        state.refresh_activity();
        state.status.assembled = true;
        if !state.poisoned && state.status.held_reason.is_none() {
            state.status.phase = if self.control.is_closing() {
                Phase::Closing
            } else {
                Phase::PendingRecovery
            };
        }
        self.publish(&state);
    }
    pub(super) fn failed_preparation(
        &self,
        error: impl Error + Send + Sync + 'static,
        acquisition: bool,
    ) -> ManagedServiceFailure {
        let reason = if acquisition {
            Held::Acquisition
        } else {
            Held::Preparation
        };
        let failure = self.record(error, reason);
        let mut state = self.state();
        state.preparation_done = true;
        state.acquisition_unknown |= acquisition;
        state.refresh_activity();
        self.publish(&state);
        drop(state);
        failure
    }
    pub(super) fn record(
        &self,
        error: impl Error + Send + Sync + 'static,
        reason: Held,
    ) -> ManagedServiceFailure {
        let mut state = self.state();
        let failure = ManagedServiceFailure {
            cause: Arc::new(error),
            reason,
        };
        state.status.phase = Phase::Held;
        state.status.held_reason = Some(reason);
        state.status.last_error = Some(failure.to_string());
        state.failures.push(failure.clone());
        self.publish(&state);
        drop(state);
        failure
    }
    pub(super) fn action(&self, jobs_empty: bool, cleanup_empty: bool) -> Action {
        let mut state = self.state();
        if state.poisoned || !self.control.is_closing() {
            return Action::Wait;
        }
        if jobs_empty
            && ((!state.registered && !state.acquisition_unknown)
                || (state.preparation_done && state.node.is_none() && !state.acquisition_unknown))
        {
            state.status.phase = Phase::Stopped;
            state.status.in_flight = false;
            self.publish(&state);
            return Action::Stop;
        }
        let Some(node) = state.node.clone() else {
            return Action::Wait;
        };
        let Some(attempts) = state.cleanup.claim(cleanup_empty) else {
            return Action::Wait;
        };
        state.refresh_activity();
        state.status.phase = Phase::Closing;
        self.publish(&state);
        Action::Drain(node, attempts)
    }
    pub(super) fn attempt(&self) {
        let mut state = self.state();
        state.status.cleanup_attempts = state.status.cleanup_attempts.saturating_add(1);
        self.publish(&state);
        drop(state);
    }
    pub(super) fn drained(&self) {
        let mut state = self.state();
        state.cleanup.finish();
        state.refresh_activity();
        self.publish(&state);
        drop(state);
    }
    pub(super) fn retry(&self) -> Result<u64, AppError> {
        let mut state = self.state();
        if state.poisoned
            || state.node.is_none()
            || state.status.held_reason != Some(Held::PhysicalCleanup)
        {
            return Err(AppError::Readiness("held managed boundary has no supported idle same-owner retry; acquisition is never replayed".into()));
        }
        if !state.cleanup.request_retry() {
            return Err(AppError::Readiness(
                "managed cleanup retry is already active or queued".into(),
            ));
        }
        self.publish(&state);
        Ok(state.status.cleanup_attempts)
    }
    pub(super) fn failure(&self) -> AppError {
        self.state().failures.last().map_or_else(
            || AppError::Readiness("managed service remains held or in flight".into()),
            |failure| crate::managed_backend::custody::failure(failure.clone()),
        )
    }
    pub(super) fn task_failed(&self, error: impl Error + Send + Sync + 'static) {
        self.record(error, Held::TaskFailure);
        let mut state = self.state();
        state.acquisition_unknown = true;
        state.poisoned = true;
        state.refresh_activity();
        self.publish(&state);
        drop(state);
    }
    pub(super) fn job_failed(&self, error: impl Error + Send + Sync + 'static, preparation: bool) {
        self.record(error, Held::TaskFailure);
        let mut state = self.state();
        if preparation {
            state.preparation_done = true;
            state.acquisition_unknown = true;
        } else {
            state.cleanup.finish();
        }
        state.refresh_activity();
        self.publish(&state);
        drop(state);
    }
    pub(super) fn cleanup_finished(&self, before: u64) -> bool {
        let state = self.state();
        state.status.cleanup_attempts > before && !state.cleanup.active
    }
}

impl State {
    const fn refresh_activity(&mut self) {
        self.status.in_flight = (self.registered && !self.preparation_done) || self.cleanup.active;
    }
}

/// Scheduling only: Registry still requires an installed actual node before claim.
#[derive(Default)]
pub(super) struct CleanupSchedule {
    active: bool,
    attempted: bool,
    retry_requested: bool,
}
impl CleanupSchedule {
    pub(super) const fn finish(&mut self) {
        self.active = false;
    }
    pub(super) const fn request_retry(&mut self) -> bool {
        if !self.attempted || self.active || self.retry_requested {
            return false;
        }
        self.retry_requested = true;
        true
    }
    pub(super) const fn claim(&mut self, cleanup_empty: bool) -> Option<usize> {
        if self.active || !cleanup_empty {
            return None;
        }
        let attempts = if !self.attempted {
            self.attempted = true;
            3
        } else if self.retry_requested {
            self.retry_requested = false;
            1
        } else {
            return None;
        };
        self.active = true;
        Some(attempts)
    }
}
