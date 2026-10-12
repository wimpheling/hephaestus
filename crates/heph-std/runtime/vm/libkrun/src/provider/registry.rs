//! Supervisor-generation inventory; observations never authorize physical cleanup.
use super::common::{
    Arc, LibkrunInstance, Lifecycle, Ordering, PendingLaunch, ProviderLifetime, VmError, VmId,
};
use super::launch::LaunchState;
use std::collections::{BTreeMap, HashSet};
use tokio::sync::Notify;

/// Diagnostic phase of one admitted VM birth in this supervisor generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmAttemptPhase {
    /// Allocation, spawn or configuration is still owned by the launch task.
    LaunchPending,
    /// An actual instance awaits the receiver's acknowledgement.
    Handoff,
    /// A configured instance has not started its guest.
    Provisioned,
    /// An admitted guest start is in progress.
    Starting,
    /// The guest was observed running.
    Running,
    /// Guest stopping has been requested.
    Stopping,
    /// Guest exit was observed; this is not physical cleanup evidence.
    ExitObserved,
    /// Startup failed; resources may still require cleanup.
    StartFailed,
    /// Destruction was requested or cleanup failed; custody is retained.
    CleanupPending,
    /// Exact process and allocation cleanup succeeded for this birth.
    Cleaned,
}

/// Immutable diagnostics for one birth; an ID can appear in multiple births.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmAttemptSnapshot {
    /// Sequence unique within this live provider generation.
    pub birth: u64,
    /// Caller-selected VM ID, which may be reused only after successful cleanup.
    pub id: VmId,
    /// Observed phase, never a drain receipt.
    pub phase: VmAttemptPhase,
    /// Original raw worker PID when this provider spawned one; diagnostic only.
    pub worker_pid: Option<u32>,
    /// Last observed launch/cleanup error, if any.
    pub error: Option<String>,
    /// Operations currently holding this instance's existing operation mutex.
    pub operations: usize,
}

/// Inventory covers this supervisor generation's admitted births only.
///
/// An empty inventory says nothing about orphans from an earlier process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmAttemptInventory {
    /// Admission is monotonic: a closed generation cannot be reopened.
    pub admission_closed: bool,
    /// Includes live, uncertain and positively cleaned births, in admission order.
    pub attempts: Vec<VmAttemptSnapshot>,
}

enum Custody {
    Pending(Arc<PendingLaunch>),
    Instance(Arc<LibkrunInstance>),
    Cleaned,
}
struct Record {
    snapshot: VmAttemptSnapshot,
    custody: Custody,
    launch_active: bool,
    lifecycle_tasks: usize,
}
struct State {
    closed: bool,
    next_birth: u64,
    keeper: bool,
    records: BTreeMap<u64, Record>,
}

impl State {
    fn settled(&self) -> bool {
        self.records.values().all(|record| {
            matches!(record.custody, Custody::Cleaned)
                && !record.launch_active
                && record.snapshot.operations == 0
                && record.lifecycle_tasks == 0
        })
    }
}

pub(super) struct Registry {
    pub(super) drain: tokio::sync::Mutex<()>,
    state: std::sync::Mutex<State>,
    changed: Notify,
}

impl Registry {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            drain: tokio::sync::Mutex::new(()),
            state: std::sync::Mutex::new(State {
                closed: false,
                next_birth: 0,
                keeper: false,
                records: BTreeMap::new(),
            }),
            changed: Notify::new(),
        })
    }

    // Caller holds the SAME lifetime.ids mutex used by close and Start.
    pub(super) fn admit(
        self: &Arc<Self>,
        lifetime: &Arc<ProviderLifetime>,
        ids: &mut HashSet<VmId>,
        entry: Arc<PendingLaunch>,
    ) -> Result<u64, VmError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(VmError::InvalidState("VM provider admission is closed"));
        }
        if ids.contains(&entry.id) {
            return Err(VmError::AlreadyExists(entry.id.clone()));
        }
        let birth = state
            .next_birth
            .checked_add(1)
            .ok_or(VmError::InvalidState("VM birth sequence exhausted"))?;
        ids.insert(entry.id.clone());
        entry.birth.store(birth, Ordering::Release);
        state.next_birth = birth;
        state.records.insert(
            birth,
            Record {
                snapshot: VmAttemptSnapshot {
                    birth,
                    id: entry.id.clone(),
                    phase: VmAttemptPhase::LaunchPending,
                    worker_pid: None,
                    error: None,
                    operations: 0,
                },
                custody: Custody::Pending(entry),
                launch_active: true,
                lifecycle_tasks: 0,
            },
        );
        if !state.keeper {
            state.keeper = true;
            // No registry -> task back-reference. This task retains original
            // supervisor custody until EVERY birth has confirmed cleanup and
            // no launch/handoff/instance operation remains. It never cleans IO.
            tokio::spawn(keep_generation(Arc::clone(self), Arc::clone(lifetime)));
        }
        drop(state);
        Ok(birth)
    }

    // Caller holds lifetime.ids; close and admission therefore linearize.
    pub(super) fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
    }

    pub(super) async fn admit_start(
        self: &Arc<Self>,
        lifetime: &ProviderLifetime,
        birth: u64,
    ) -> Result<Operation, VmError> {
        let ids = lifetime.ids.lock().await;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(VmError::InvalidState("VM provider admission is closed"));
        }
        let record = state
            .records
            .get_mut(&birth)
            .ok_or(VmError::InvalidState("VM birth is not registered"))?;
        if matches!(record.custody, Custody::Cleaned) {
            return Err(VmError::Destroyed);
        }
        record.snapshot.operations += 1;
        drop(state);
        drop(ids);
        Ok(Operation {
            registry: Arc::clone(self),
            birth,
            kind: Activity::Operation,
        })
    }

    pub(super) fn lifecycle_task(self: &Arc<Self>, birth: u64) -> Result<Operation, VmError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let record = state
            .records
            .get_mut(&birth)
            .ok_or(VmError::InvalidState("VM birth is not registered"))?;
        record.lifecycle_tasks = record
            .lifecycle_tasks
            .checked_add(1)
            .ok_or(VmError::InvalidState("VM lifecycle task count exhausted"))?;
        drop(state);
        Ok(Operation {
            registry: Arc::clone(self),
            birth,
            kind: Activity::LifecycleTask,
        })
    }

    pub(super) fn operation(self: &Arc<Self>, birth: u64) -> Operation {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&birth) {
            record.snapshot.operations += 1;
        }
        drop(state);
        Operation {
            registry: Arc::clone(self),
            birth,
            kind: Activity::Operation,
        }
    }

    pub(super) fn install(&self, birth: u64, instance: Arc<LibkrunInstance>, pid: Option<u32>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&birth) {
            record.snapshot.worker_pid = pid;
            record.snapshot.phase = VmAttemptPhase::Handoff;
            record.custody = Custody::Instance(instance);
        }
    }

    pub(super) fn launch_finished(&self, birth: u64, cleaned: bool, pid: Option<u32>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&birth) {
            record.launch_active = false;
            record.snapshot.worker_pid = pid.or(record.snapshot.worker_pid);
            if cleaned {
                record.snapshot.phase = VmAttemptPhase::Cleaned;
                record.snapshot.error = None;
                record.custody = Custody::Cleaned;
            }
        }
        drop(state);
        self.changed.notify_waiters();
    }

    pub(super) fn cleaned(&self, birth: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&birth) {
            record.snapshot.phase = VmAttemptPhase::Cleaned;
            record.snapshot.error = None;
            record.custody = Custody::Cleaned;
        }
        drop(state);
        self.changed.notify_waiters();
    }

    pub(super) fn cleanup_failed(&self, birth: u64, error: &VmError) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&birth) {
            record.snapshot.error = Some(error.to_string());
        }
    }

    pub(super) fn instances(&self) -> Vec<Arc<LibkrunInstance>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .records
            .values()
            .filter_map(|record| match &record.custody {
                Custody::Instance(instance) => Some(Arc::clone(instance)),
                Custody::Pending(_) | Custody::Cleaned => None,
            })
            .collect()
    }

    // Caller holds lifetime.ids, preserving the existing ids -> state order.
    // The synchronous observer must never reenter registry/ids or perform cleanup.
    pub(super) fn observe_settled<T>(
        &self,
        inspect: impl FnOnce() -> Result<T, VmError>,
    ) -> Result<T, VmError> {
        let state = self
            .state
            .lock()
            .map_err(|_| VmError::InvalidState("VM birth registry is poisoned"))?;
        if !state.settled() {
            return Err(VmError::InvalidState(
                "live VM generation remains unsettled",
            ));
        }
        let result = inspect();
        drop(state);
        result
    }

    pub(super) async fn wait_settled(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .settled()
            {
                return;
            }
            changed.await;
        }
    }

    pub(super) async fn inventory(&self) -> VmAttemptInventory {
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

// Activity bookkeeping, not another operation lock or cleanup evidence.
// LifecycleTask additionally covers tasks queued before the operation mutex.
enum Activity {
    Operation,
    LifecycleTask,
}
pub(super) struct Operation {
    registry: Arc<Registry>,
    birth: u64,
    kind: Activity,
}
impl Drop for Operation {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&self.birth) {
            match self.kind {
                Activity::Operation => record.snapshot.operations -= 1,
                Activity::LifecycleTask => record.lifecycle_tasks -= 1,
            }
        }
        drop(state);
        self.registry.changed.notify_waiters();
    }
}

async fn keep_generation(registry: Arc<Registry>, lifetime: Arc<ProviderLifetime>) {
    loop {
        let changed = registry.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let complete = {
            let ids = lifetime.ids.lock().await;
            let mut state = registry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let complete = state.settled();
            if complete {
                state.keeper = false;
            }
            drop(state);
            drop(ids);
            complete
        };
        if complete {
            return;
        }
        changed.await;
    }
}
