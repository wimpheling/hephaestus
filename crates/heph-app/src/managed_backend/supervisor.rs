//! Explicit daemon-owned service; no detached anonymous construction park.
#[path = "supervisor/jobs.rs"]
mod jobs;
#[path = "supervisor/registry.rs"]
mod registry;
#[path = "supervisor/status.rs"]
mod status;
#[cfg(test)]
#[path = "supervisor/tests.rs"]
mod tests;

use crate::{AppConfig, AppError, ManagedInvocationPools};
pub use status::{ManagedHeldReason, ManagedSupervisorPhase, ManagedSupervisorStatus};
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Notify, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
};

#[derive(Default)]
struct Control {
    closing: AtomicBool,
    changed: Notify,
}
impl Control {
    fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }
    fn close(&self) {
        self.closing.store(true, Ordering::Release);
        self.changed.notify_one();
    }
}

/// Private transferred lease; abandonment requests close but never owns originals.
pub struct NodeLease {
    control: Arc<Control>,
    status: watch::Receiver<ManagedSupervisorStatus>,
}
impl NodeLease {
    /// Managed startup remains pending until separate lifecycle qualification.
    /// # Errors
    /// Always returns the current pending assembly/recovery reason.
    pub fn require_ready(&self) -> Result<(), AppError> {
        super::custody::pending_readiness(self.status.borrow().assembled)
    }
    /// Monotonically requests service-owned closing without claiming stop.
    pub fn close(&self) {
        self.control.close();
    }
}
impl Drop for NodeLease {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) struct PreparationTicket {
    lease: Option<NodeLease>,
    result: oneshot::Receiver<Result<(), ManagedServiceFailure>>,
}
impl PreparationTicket {
    pub(super) async fn wait(mut self) -> Result<NodeLease, AppError> {
        (&mut self.result)
            .await
            .map_err(super::custody::failure)?
            .map_err(super::custody::failure)?;
        Ok(self.lease.take().expect("registered ticket owns its lease"))
    }
}

#[derive(Clone, Debug)]
struct ManagedServiceFailure {
    cause: Arc<dyn Error + Send + Sync>,
    reason: ManagedHeldReason,
}
impl fmt::Display for ManagedServiceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "managed service remains held ({:?}): {}",
            self.reason, self.cause
        )
    }
}
impl Error for ManagedServiceFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

/// Non-clone daemon owner of original managed preparation and closing custody.
///
/// Retain this service and its Tokio runtime until confirmed Stopped, including
/// build/start errors. Drop requests close without awaiting; abrupt runtime or
/// process termination is unconfirmed, never a cleanup or absence guarantee.
pub struct ManagedBackendSupervisor {
    registry: Arc<registry::Registry>,
    status: watch::Receiver<ManagedSupervisorStatus>,
    requests: mpsc::UnboundedSender<jobs::Preparation>,
    task: Option<JoinHandle<()>>,
}
impl ManagedBackendSupervisor {
    /// Starts the named process-owned service in the current Tokio runtime.
    ///
    /// Acquires no roots. The service itself must outlive abandoned App/error waiters.
    /// # Panics
    /// Panics if called without an active Tokio runtime.
    #[must_use]
    pub fn start() -> Self {
        let (registry, status) = registry::Registry::new();
        let (requests, receiver) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(Arc::clone(&registry), receiver));
        Self {
            registry,
            status,
            requests,
            task: Some(task),
        }
    }
    /// Returns diagnostics, never owner handles or readiness authority.
    #[must_use]
    pub fn status(&self) -> ManagedSupervisorStatus {
        let mut status = self.status.borrow().clone();
        if self.registry.control.is_closing()
            && matches!(
                status.phase,
                ManagedSupervisorPhase::Preparing
                    | ManagedSupervisorPhase::Constructing
                    | ManagedSupervisorPhase::PendingRecovery
            )
        {
            status.phase = ManagedSupervisorPhase::Closing;
        }
        status
    }
    /// Awaits diagnostic change; cancellation discards only this waiter.
    /// # Errors
    /// Returns an error if the diagnostic channel unexpectedly closes.
    pub async fn changed(&mut self) -> Result<ManagedSupervisorStatus, AppError> {
        self.status
            .changed()
            .await
            .map_err(super::custody::failure)?;
        Ok(self.status())
    }
    pub(super) fn register(
        &self,
        app: AppConfig,
        pools: ManagedInvocationPools,
    ) -> Result<PreparationTicket, AppError> {
        let inputs = Arc::new(jobs::PreparationInputs { app, pools });
        let lease = self
            .registry
            .register(Arc::clone(&inputs), self.status.clone())?;
        let (result, receiver) = oneshot::channel();
        if self
            .requests
            .send(jobs::Preparation { inputs, result })
            .is_err()
        {
            let cause = self.registry.failed_preparation(
                AppError::Task("managed service request channel closed".into()),
                false,
            );
            return Err(super::custody::failure(cause));
        }
        Ok(PreparationTicket {
            lease: Some(lease),
            result: receiver,
        })
    }
    /// Requests one supported idle SAME-owner cleanup pass and observes its outcome.
    ///
    /// Does not replay acquisition/bootstrap, reset automatic budgets or authorize IO.
    /// # Errors
    /// Rejects unsupported/in-flight held boundaries or returns retained cleanup failure.
    pub async fn retry_held(&self) -> Result<(), AppError> {
        let before = self.registry.retry()?;
        let mut status = self.status.clone();
        loop {
            if self.registry.cleanup_finished(before) {
                return Err(self.registry.failure()); // Reconciliation is not wired yet.
            }
            status.changed().await.map_err(super::custody::failure)?;
        }
    }
    /// Seals intake and observes confirmed Stopped before joining the service.
    ///
    /// Deadline/cancellation never aborts jobs or releases originals. Retain this
    /// borrowed service/runtime on Held and use explicit supported retry.
    /// # Errors
    /// Returns timeout or actual retained failures; missing reconciliation stays Held.
    pub async fn shutdown(&mut self, timeout: Duration) -> Result<(), AppError> {
        self.registry.control.close();
        tokio::time::timeout(timeout, self.join_stopped())
            .await
            .map_err(|_| {
                AppError::Timeout(
                    "managed service remains held or in flight; original custody retained".into(),
                )
            })?
    }
    async fn join_stopped(&mut self) -> Result<(), AppError> {
        loop {
            if self.status().phase == ManagedSupervisorPhase::Stopped {
                if let Some(task) = self.task.as_mut() {
                    if let Err(cause) = task.await {
                        self.registry.task_failed(cause);
                        self.task.take();
                        return Err(self.registry.failure());
                    }
                    self.task.take();
                }
                return Ok(());
            }
            if self.task.as_ref().is_some_and(JoinHandle::is_finished) {
                let task = self.task.take().expect("finished task is retained");
                match task.await {
                    Ok(()) => self.registry.task_failed(AppError::Task(
                        "managed service exited without Stopped proof".into(),
                    )),
                    Err(cause) => self.registry.task_failed(cause),
                }
                return Err(self.registry.failure());
            }
            // Held without an active pass has a concrete retained error, not success.
            if self.status().phase == ManagedSupervisorPhase::Held && !self.status().in_flight {
                return Err(self.registry.failure());
            }
            self.changed().await?;
        }
    }
}
impl Drop for ManagedBackendSupervisor {
    fn drop(&mut self) {
        self.registry.control.close();
        // JoinHandle Drop detaches, never aborts. The named loop retains registry
        // and jobs while its runtime lives; Drop itself proves nothing stopped.
    }
}

async fn run(
    registry: Arc<registry::Registry>,
    mut requests: mpsc::UnboundedReceiver<jobs::Preparation>,
) {
    let mut preparation = JoinSet::new();
    let mut cleanup = JoinSet::new();
    let mut connected = true;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    loop {
        let wake = registry.control.changed.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        match registry.action(
            preparation.is_empty() && cleanup.is_empty(),
            cleanup.is_empty(),
        ) {
            registry::Action::Wait => {}
            registry::Action::Stop => return,
            registry::Action::Drain(node, attempts) => {
                cleanup.spawn(Box::pin(jobs::drain(Arc::clone(&registry), node, attempts)));
            }
        }
        tokio::select! {
            () = wake => {}
            request = requests.recv(), if connected => {
                if let Some(request) = request {
                    preparation.spawn(Box::pin(jobs::prepare(Arc::clone(&registry), request)));
                } else {
                    connected = false;
                    registry.control.close();
                }
            },
            outcome = preparation.join_next(), if !preparation.is_empty() => {
                if let Some(Err(cause)) = outcome { registry.job_failed(cause, true); }
            }
            outcome = cleanup.join_next(), if !cleanup.is_empty() => {
                if let Some(Err(cause)) = outcome { registry.job_failed(cause, false); }
            }
            _ = heartbeat.tick() => {
                tracing::info!(closing = registry.control.is_closing(), "managed supervisor retains original service custody");
            }
        }
    }
}
