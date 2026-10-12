//! Retained partial launch ownership; no global admission or drain receipt.
use super::common::{
    Arc, AtomicBool, AtomicU64, Cgroup, Command, Duration, EVENT_CAPACITY, HashMap, LibkrunConfig,
    LibkrunInstance, Lifecycle, Mutex, OwnedResources, PathBuf, PreparedSpec, ProviderInner,
    Semaphore, VmError, VmId, WorkerBackend, WorkerProcess, broadcast, oneshot, timeout, watch,
};
use super::{
    helpers::{cleanup_runtime, create_runtime_dir_tracked, provider_error, unavailable_error},
    ownership::{OwnerGuard, PinnedCleanupRoots},
};
use std::{fmt, os::fd::AsRawFd};
use tokio::sync::mpsc;

type CleanupReply = oneshot::Sender<Result<(), VmError>>;
pub(super) type RetryInbox = mpsc::Receiver<CleanupReply>;

pub(super) struct LaunchCustody {
    pub(super) host_runtime_dir: PathBuf,
    process: std::sync::Mutex<Option<Arc<WorkerProcess>>>,
    backend: std::sync::Mutex<Option<Arc<dyn WorkerBackend>>>,
}

impl LaunchCustody {
    fn new(host_runtime_dir: PathBuf) -> Self {
        Self {
            host_runtime_dir,
            process: std::sync::Mutex::new(None),
            backend: std::sync::Mutex::new(None),
        }
    }

    pub(super) fn spawn(&self, command: &mut Command) -> Result<Arc<WorkerProcess>, VmError> {
        let mut slot = self.process.lock().map_err(|error| custody_error(&error))?;
        if slot.is_some()
            || self
                .backend
                .lock()
                .map_err(|error| custody_error(&error))?
                .is_some()
        {
            return Err(VmError::InvalidState("launch already owns a worker"));
        }
        let child = command
            .spawn()
            .map_err(|error| unavailable_error("worker binary", error.to_string()))?;
        let process = WorkerProcess::retain(child);
        // No await separates spawn from installation into the caller's owner.
        *slot = Some(Arc::clone(&process));
        drop(slot);
        Ok(process)
    }

    pub(super) fn record_backend(&self, backend: Arc<dyn WorkerBackend>) -> Result<(), VmError> {
        let mut slot = self.backend.lock().map_err(|error| custody_error(&error))?;
        if let Some(current) = slot.as_ref() {
            if !Arc::ptr_eq(current, &backend) {
                return Err(VmError::InvalidState("launch backend identity changed"));
            }
        } else {
            *slot = Some(backend);
        }
        drop(slot);
        Ok(())
    }

    pub(super) fn process(&self) -> Result<Option<Arc<WorkerProcess>>, VmError> {
        Ok(self
            .process
            .lock()
            .map_err(|error| custody_error(&error))?
            .clone())
    }

    pub(super) async fn terminate(&self, grace: Duration) -> Result<(), VmError> {
        let backend = self
            .backend
            .lock()
            .map_err(|error| custody_error(&error))?
            .clone();
        if let Some(backend) = backend {
            backend.kill().await?;
            timeout(grace, backend.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
            return Ok(());
        }
        let process = self.process()?;
        if let Some(process) = process {
            process.kill().await?;
            timeout(grace, process.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        }
        Ok(())
    }
}

fn custody_error(error: &impl fmt::Display) -> VmError {
    provider_error("launch-custody", std::io::Error::other(error.to_string()))
}

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
    runtime: Option<PathBuf>,
    cgroup: Option<Cgroup>,
    broker: Option<Arc<crate::service_transport::ServiceBroker>>,
    roots: Option<PinnedCleanupRoots>,
}

pub(super) struct PendingLaunch {
    pub(super) id: VmId,
    pub(super) custody: LaunchCustody,
    pub(super) state: watch::Sender<LaunchState>,
    resources: Mutex<LaunchResources>,
    retry: mpsc::Sender<CleanupReply>,
}

impl PendingLaunch {
    pub(super) fn new(
        id: VmId,
        config: &LibkrunConfig,
        guard: Option<&OwnerGuard>,
    ) -> Result<(Arc<Self>, RetryInbox), VmError> {
        let roots = guard.map(PinnedCleanupRoots::from_guard).transpose()?;
        // The host socket path uses the cloned cleanup descriptor, whose
        // lifetime continues with the installed instance after OwnerGuard ends.
        let host_runtime = roots.as_ref().map_or_else(
            || config.runtime_root.join(&id.0),
            |roots| roots.runtime_path(&id),
        );
        let (retry, receiver) = mpsc::channel(1);
        Ok((
            Arc::new(Self {
                custody: LaunchCustody::new(host_runtime),
                id,
                state: watch::channel(LaunchState::Working).0,
                resources: Mutex::new(LaunchResources {
                    runtime: None,
                    cgroup: None,
                    broker: None,
                    roots,
                }),
                retry,
            }),
            receiver,
        ))
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

    async fn launch(
        &self,
        provider: &ProviderInner,
        spec: PreparedSpec,
        guard: Option<&OwnerGuard>,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        if let Some((owner, guard)) = provider.owner.get().zip(guard) {
            owner.validate_guard(&provider.config, guard)?;
        }
        let mut allocation = provider.config.as_ref().clone();
        if let Some(guard) = guard {
            allocation.runtime_root = format!("/proc/self/fd/{}", guard.runtime.as_raw_fd()).into();
            allocation.cgroup_root = format!("/proc/self/fd/{}", guard.cgroup.as_raw_fd()).into();
        }
        let mut resources = self.resources.lock().await;
        let runtime = provider.config.runtime_root.join(&self.id.0);
        create_runtime_dir_tracked(&allocation.runtime_root, &self.id.0, |_| {
            resources.runtime = Some(runtime.clone());
        })?;
        Cgroup::create_tracked(&allocation, &self.id.0, |_| {
            resources.cgroup = Some(Cgroup::existing(&provider.config, &self.id.0));
        })?;
        if let Some(service) = spec.private_http_service.as_ref() {
            let duration = Duration::from_millis(service.connect_timeout_ms);
            resources.broker = Some(Arc::new(
                crate::service_transport::ServiceBroker::bind(
                    &self.custody.host_runtime_dir,
                    service.max_connections,
                    duration,
                )
                .map_err(|error| unavailable_error("private service broker", error.to_string()))?,
            ));
        }
        drop(resources);
        // Placement uses the retained host root; worker configuration keeps
        // its original paths, whose later redirection remains a separate gap.
        let placement = Cgroup::existing(&allocation, &self.id.0);
        let backend = provider
            .worker_spawner
            .spawn(
                Arc::clone(&provider.config),
                spec,
                &runtime,
                &placement,
                &self.custody,
            )
            .await?;
        self.custody.record_backend(Arc::clone(&backend))?;
        if let Some((owner, guard)) = provider.owner.get().zip(guard) {
            owner.validate_guard(&provider.config, guard)?;
        }
        Ok(backend)
    }

    async fn install_instance(
        &self,
        provider: &Arc<ProviderInner>,
        worker: Arc<dyn WorkerBackend>,
        settings: InstanceSettings,
    ) -> Result<Arc<LibkrunInstance>, VmError> {
        let mut resources = self.resources.lock().await;
        let runtime_dir = resources
            .runtime
            .as_ref()
            .ok_or(VmError::InvalidState("launch runtime allocation missing"))?
            .clone();
        let cgroup = resources
            .cgroup
            .take()
            .ok_or(VmError::InvalidState("launch cgroup allocation missing"))?;
        resources.runtime = None;
        let owned = OwnedResources {
            runtime_dir,
            cgroup,
            service_broker: resources.broker.take(),
            cleanup_roots: resources.roots.take(),
        };
        drop(resources);
        let instance = Arc::new(LibkrunInstance {
            id: self.id.clone(),
            config: Arc::clone(&provider.config),
            worker,
            state: Mutex::new(Lifecycle::Provisioned),
            operation: Mutex::new(()),
            destroy_requested: watch::channel(false).0,
            stop_requested: watch::channel(false).0,
            cleanup_complete: AtomicBool::new(false),
            terminal: watch::channel(None).0,
            terminal_guard: Mutex::new(()),
            ready: watch::channel(false).0,
            start_result: watch::channel(None).0,
            events: broadcast::channel(EVENT_CAPACITY).0,
            resources: Mutex::new(Some(owned)),
            provider_ids: Arc::clone(provider),
            private_http_enabled: settings.http_enabled,
            private_service_timeout: settings.service_timeout,
            private_service_dispatch: settings.dispatch,
            private_http_waiters: Mutex::new(HashMap::new()),
            next_private_http_request: AtomicU64::new(1),
        });
        instance.spawn_event_forwarder();
        instance.spawn_process_monitor();
        Ok(instance)
    }

    async fn cleanup(&self, provider: &ProviderInner) -> Result<(), VmError> {
        self.custody
            .terminate(provider.config.startup_timeout)
            .await?;
        let mut resources = self.resources.lock().await;
        if let Some(broker) = resources.broker.as_ref() {
            broker.shutdown().await;
        }
        if let Some(cgroup) = resources.cgroup.as_ref() {
            if let Some(roots) = resources.roots.as_ref() {
                roots.cleanup_cgroup(&provider.config, &self.id)?;
            } else {
                cgroup.cleanup()?;
            }
        }
        if let Some(runtime) = resources.runtime.as_ref() {
            if let Some(roots) = resources.roots.as_ref() {
                roots.cleanup_runtime(&self.id)?;
            } else {
                cleanup_runtime(runtime)?;
            }
        }
        resources.runtime = None;
        resources.cgroup = None;
        resources.broker = None;
        drop(resources);
        Ok(())
    }
}

pub(super) struct InstanceSettings {
    http_enabled: bool,
    service_timeout: Option<Duration>,
    dispatch: Option<Arc<Semaphore>>,
}

impl InstanceSettings {
    pub(super) fn from_spec(spec: &PreparedSpec) -> Self {
        Self {
            http_enabled: spec
                .labels
                .get(crate::protocol::GATEWAY_HANDLER_CONTRACT_LABEL)
                .is_some_and(|value| value == crate::protocol::GATEWAY_HANDLER_CONTRACT_V1),
            service_timeout: spec
                .private_http_service
                .as_ref()
                .map(|service| Duration::from_millis(service.connect_timeout_ms)),
            dispatch: private_service_dispatch(spec),
        }
    }
}

// Preserve the reviewed pure extraction from the strict-lint successor.
fn private_service_dispatch(spec: &PreparedSpec) -> Option<Arc<Semaphore>> {
    spec.private_http_service
        .as_ref()
        .map(|service| Arc::new(Semaphore::new(service.max_connections as usize)))
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
    let launched = tokio::select! {
        launched = entry.launch(&provider, spec, guard.as_ref()) => launched,
        () = result.closed() => Err(unavailable_error("worker launch", "provision receiver cancelled")),
    };
    match launched {
        Ok(worker) => match entry.install_instance(&provider, worker, settings).await {
            Ok(instance) => {
                entry.state.send_replace(LaunchState::Handoff);
                let (accepted, acknowledgement) = oneshot::channel();
                let sent = result
                    .send(Ok(LaunchHandoff {
                        instance: Arc::clone(&instance),
                        accepted,
                    }))
                    .is_ok();
                if sent && acknowledgement.await.is_ok() {
                    entry.state.send_replace(LaunchState::HandedOff);
                    retire(&provider, &entry, false).await;
                    return;
                }
                hold_or_retire(
                    provider,
                    entry,
                    guard,
                    Some(instance),
                    unavailable_error("worker launch", "provision handoff cancelled"),
                    None,
                    retries,
                )
                .await;
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

async fn cleanup_entry(
    provider: &ProviderInner,
    entry: &PendingLaunch,
    instance: Option<&Arc<LibkrunInstance>>,
) -> Result<(), VmError> {
    if let Some(instance) = instance {
        vm_trait::VmInstance::destroy(instance.as_ref()).await
    } else {
        entry.cleanup(provider).await
    }
}

async fn retire(provider: &ProviderInner, entry: &PendingLaunch, release_id: bool) {
    let mut ids = provider.ids.lock().await;
    // No await between releasing the exact reservation and retiring its entry.
    let mut pending = provider
        .pending_launches
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pending
        .get(&entry.id)
        .is_some_and(|current| std::ptr::eq(current.as_ref(), entry))
    {
        if release_id {
            ids.remove(&entry.id);
        }
        pending.remove(&entry.id);
    }
    drop(pending);
    drop(ids);
}

async fn hold_or_retire(
    provider: Arc<ProviderInner>,
    entry: Arc<PendingLaunch>,
    guard: Option<OwnerGuard>,
    instance: Option<Arc<LibkrunInstance>>,
    cause: VmError,
    result: Option<oneshot::Sender<Result<LaunchHandoff, VmError>>>,
    mut retries: RetryInbox,
) {
    let cleanup = cleanup_entry(&provider, &entry, instance.as_ref()).await;
    let cause = match cleanup {
        Ok(()) => {
            retire(&provider, &entry, true).await;
            entry.state.send_replace(LaunchState::Cleaned);
            if let Some(result) = result {
                let _sent = result.send(Err(cause));
            }
            return;
        }
        Err(cleanup) => {
            let cause = Arc::new(cause);
            let failure = Arc::new(LaunchFailure {
                cause: Arc::clone(&cause),
                cleanup: Arc::new(cleanup),
            });
            entry
                .state
                .send_replace(LaunchState::Held(Arc::clone(&failure)));
            if let Some(result) = result {
                let _sent = result.send(Err(provider_error("worker-launch-cleanup", failure)));
            }
            cause
        }
    };
    // The task retains original provider/owner custody during uncertainty. The
    // map entry has no Arc back-reference and no public abort handle.
    while let Some(reply) = retries.recv().await {
        entry.state.send_replace(LaunchState::Working);
        match cleanup_entry(&provider, &entry, instance.as_ref()).await {
            Ok(()) => {
                retire(&provider, &entry, true).await;
                entry.state.send_replace(LaunchState::Cleaned);
                let _sent = reply.send(Ok(()));
                drop(guard);
                return;
            }
            Err(cleanup) => {
                let failure = Arc::new(LaunchFailure {
                    cause: Arc::clone(&cause),
                    cleanup: Arc::new(cleanup),
                });
                entry
                    .state
                    .send_replace(LaunchState::Held(Arc::clone(&failure)));
                let _sent = reply.send(Err(provider_error("worker-launch-cleanup", failure)));
            }
        }
    }
}
