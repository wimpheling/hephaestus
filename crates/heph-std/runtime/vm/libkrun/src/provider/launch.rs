//! Retained partial launch ownership; no global admission or drain receipt.
use super::common::{
    Arc, AtomicU64, Cgroup, LibkrunConfig, LibkrunInstance, Mutex, PreparedSpec, ProviderInner,
    VmError, VmId, oneshot, watch,
};
use super::{
    common, containment,
    helpers::{provider_error, unavailable_error},
    ownership::{OwnerGuard, PinnedCleanupRoots},
};
use std::fmt;
use tokio::sync::mpsc;

#[path = "launch/custody.rs"]
mod custody;
#[path = "launch/resources.rs"]
mod resources;
#[path = "launch/retirement.rs"]
mod retirement;
pub(super) use custody::LaunchCustody;
pub(super) use resources::InstanceSettings;
use retirement::{hold_or_retire, retire};

type CleanupReply = oneshot::Sender<Result<(), VmError>>;
pub(super) type RetryInbox = mpsc::Receiver<CleanupReply>;

#[derive(Debug)]
pub(super) struct LaunchFailure {
    pub(super) cause: Arc<VmError>,
    pub(super) cleanup: Arc<VmError>,
}

impl fmt::Display for LaunchFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "launch failed: {}; cleanup remains unresolved: {}",
            self.cause, self.cleanup
        )
    }
}
impl std::error::Error for LaunchFailure {}

#[derive(Clone)]
pub(super) enum LaunchState {
    Working,
    Handoff,
    Held(Arc<LaunchFailure>),
    Cleaned,
    HandedOff,
}

pub(super) struct LaunchHandoff {
    pub(super) instance: Arc<LibkrunInstance>,
    pub(super) accepted: oneshot::Sender<()>,
}

struct LaunchResources {
    runtime: Option<crate::directory::ChildDirectory>,
    cgroup: Option<Cgroup>,
    broker: Option<Arc<crate::service_transport::ServiceBroker>>,
    birth_record: Option<Arc<super::containment::BirthRecord>>,
    roots: Option<PinnedCleanupRoots>,
}

pub(super) struct PendingLaunch {
    pub(super) id: VmId,
    pub(super) birth: AtomicU64,
    pub(super) custody: LaunchCustody,
    pub(super) state: watch::Sender<LaunchState>,
    resources: Mutex<LaunchResources>,
    retry: mpsc::Sender<CleanupReply>,
    drain: watch::Sender<bool>,
}

impl PendingLaunch {
    pub(super) fn new(
        id: VmId,
        _config: &LibkrunConfig,
        guard: Option<&OwnerGuard>,
    ) -> Result<(Arc<Self>, RetryInbox), VmError> {
        let roots = guard.map(PinnedCleanupRoots::from_guard).transpose()?;
        let (retry, receiver) = mpsc::channel(1);
        Ok((
            Arc::new(Self {
                custody: LaunchCustody::new(),
                id,
                birth: AtomicU64::new(0),
                state: watch::channel(LaunchState::Working).0,
                resources: Mutex::new(LaunchResources {
                    runtime: None,
                    cgroup: None,
                    broker: None,
                    birth_record: None,
                    roots,
                }),
                retry,
                drain: watch::channel(false).0,
            }),
            receiver,
        ))
    }

    // Called while the provider's admission/acknowledgement IDs lock is held.
    pub(super) fn request_drain(&self) {
        self.drain.send_replace(true);
    }

    pub(super) fn drain_requested(&self) -> bool {
        *self.drain.borrow()
    }

    pub(super) async fn drain_cleanup(&self, retry_held: bool) -> Result<(), VmError> {
        // Only the drain's pre-cancellation snapshot authorizes an explicit
        // retry. A newly Held failure keeps its first original cause intact.
        if retry_held {
            return self.retry_cleanup().await;
        }
        let mut state = self.state.subscribe();
        loop {
            let current = state.borrow_and_update().clone();
            match current {
                LaunchState::Cleaned | LaunchState::HandedOff => return Ok(()),
                LaunchState::Held(failure) => {
                    return Err(provider_error("worker-launch-cleanup", failure));
                }
                LaunchState::Working | LaunchState::Handoff => {}
            }
            state
                .changed()
                .await
                .map_err(|error| provider_error("launch-drain-state", error))?;
        }
    }

    pub(super) async fn retry_cleanup(&self) -> Result<(), VmError> {
        let state = self.state.borrow().clone();
        match state {
            LaunchState::Cleaned => return Ok(()),
            LaunchState::Held(_) => {}
            LaunchState::Working | LaunchState::Handoff | LaunchState::HandedOff => {
                return Err(VmError::InvalidState(
                    "pending launch is not held for cleanup",
                ));
            }
        }
        let (sender, receiver) = oneshot::channel();
        if self.retry.send(sender).await.is_err() {
            return self.completed_or_error("launch cleanup owner channel closed");
        }
        receiver
            .await
            .unwrap_or_else(|_| self.completed_or_error("launch cleanup reply channel closed"))
    }

    fn completed_or_error(&self, message: &'static str) -> Result<(), VmError> {
        let state = self.state.borrow().clone();
        match state {
            LaunchState::Cleaned => Ok(()),
            LaunchState::Held(failure) => Err(provider_error("worker-launch-cleanup", failure)),
            LaunchState::Working | LaunchState::Handoff | LaunchState::HandedOff => {
                Err(unavailable_error("worker launch custody", message))
            }
        }
    }
}

pub(super) async fn run_launch(
    provider: Arc<ProviderInner>,
    entry: Arc<PendingLaunch>,
    spec: PreparedSpec,
    guard: Option<OwnerGuard>,
    mut result: oneshot::Sender<Result<LaunchHandoff, VmError>>,
    retries: RetryInbox,
) {
    let settings = InstanceSettings::from_spec(&spec);
    let mut draining = entry.drain.subscribe();
    let launched = tokio::select! {
        biased;
        _requested = draining.wait_for(|requested| *requested) => Err(unavailable_error("worker launch", "generation drain requested")),
        launched = entry.launch(&provider, spec, guard.as_ref()) => launched,
        () = result.closed() => Err(unavailable_error("worker launch", "provision receiver cancelled")),
    };
    match launched {
        Ok(worker) => match entry.install_instance(&provider, worker, settings).await {
            Ok(instance) => {
                if entry.drain_requested() {
                    hold_or_retire(
                        provider,
                        entry,
                        guard,
                        Some(instance),
                        unavailable_error("worker launch", "generation drain requested"),
                        Some(result),
                        retries,
                    )
                    .await;
                    return;
                }
                entry.state.send_replace(LaunchState::Handoff);
                let (accepted, acknowledgement) = oneshot::channel();
                let sent = result
                    .send(Ok(LaunchHandoff {
                        instance: Arc::clone(&instance),
                        accepted,
                    }))
                    .is_ok();
                let acknowledged = sent
                    && tokio::select! {
                        biased;
                        _requested = draining.wait_for(|requested| *requested) => false,
                        outcome = acknowledgement => outcome.is_ok(),
                    };
                if acknowledged {
                    entry.state.send_replace(LaunchState::HandedOff);
                    retire(&provider, &entry, false).await;
                    return;
                }
                let cause = unavailable_error(
                    "worker launch",
                    if entry.drain_requested() {
                        "generation drain requested"
                    } else {
                        "provision handoff cancelled"
                    },
                );
                hold_or_retire(provider, entry, guard, Some(instance), cause, None, retries).await;
            }
            Err(cause) => {
                hold_or_retire(provider, entry, guard, None, cause, Some(result), retries).await;
            }
        },
        Err(cause) => {
            hold_or_retire(provider, entry, guard, None, cause, Some(result), retries).await;
        }
    }
}
