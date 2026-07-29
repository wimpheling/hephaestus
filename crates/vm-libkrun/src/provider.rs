use crate::{
    cgroup::Cgroup,
    config::LibkrunConfig,
    framing::{read_async, write_async},
    validation::{
        PROVIDER_NAME, PreparedForward, PreparedSpec, prepare_spec, validate_config, validate_id,
    },
    worker::{
        WireError, WireErrorKind, WireLogStream, WorkerCommand, WorkerConfiguration, WorkerEvent,
        WorkerMessage, WorkerRequest,
    },
};
use async_trait::async_trait;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt, fs, io,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{UnixListener, unix::OwnedReadHalf, unix::OwnedWriteHalf},
    process::{Child, Command},
    sync::{Mutex, broadcast, oneshot, watch},
    time::{sleep, timeout},
};
use tracing::{error, info, warn};
use vm_trait::{
    LogStream, PortForward, PortProtocol, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance,
    VmMetric, VmProvider, VmSpec,
};

const EVENT_CAPACITY: usize = 256;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Fedora/Linux VM provider backed by a dedicated libkrun worker per VM.
#[derive(Clone)]
pub struct LibkrunProvider {
    inner: Arc<ProviderInner>,
}

struct ProviderInner {
    config: Arc<LibkrunConfig>,
    ids: Mutex<HashSet<VmId>>,
    worker_spawner: Arc<dyn WorkerSpawner>,
}

impl LibkrunProvider {
    /// Validates host configuration and constructs a provider.
    ///
    /// This checks paths, the delegated cgroup, effective service identity,
    /// executable availability, and `/dev/kvm` access. libkrun/libkrunfw are
    /// loaded by each dedicated worker during provisioning.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidSpec`] for invalid configuration and
    /// [`VmError::Unavailable`] when a required host resource is unavailable.
    pub fn new(config: LibkrunConfig) -> Result<Self, VmError> {
        Self::new_with_spawner(config, Arc::new(ProcessWorkerSpawner))
    }

    fn new_with_spawner(
        config: LibkrunConfig,
        worker_spawner: Arc<dyn WorkerSpawner>,
    ) -> Result<Self, VmError> {
        validate_config(&config)?;
        Ok(Self {
            inner: Arc::new(ProviderInner {
                config: Arc::new(config),
                ids: Mutex::new(HashSet::new()),
                worker_spawner,
            }),
        })
    }
}

#[async_trait]
impl VmProvider for LibkrunProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let prepared = prepare_spec(&self.inner.config, &spec)?;
        {
            let mut ids = self.inner.ids.lock().await;
            if !ids.insert(spec.id.clone()) {
                return Err(VmError::AlreadyExists(spec.id));
            }
        }

        match self.provision_inner(spec.id.clone(), prepared).await {
            Ok(instance) => Ok(instance),
            Err(error) => {
                self.inner.ids.lock().await.remove(&spec.id);
                Err(error)
            }
        }
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        validate_id(id)?;
        if self.inner.ids.lock().await.contains(id) {
            return Err(VmError::InvalidState(
                "cannot clean an orphan while a live instance handle is registered",
            ));
        }

        Cgroup::existing(&self.inner.config, &id.0).cleanup()?;
        cleanup_runtime(&self.inner.config.runtime_root.join(&id.0))
    }
}

impl LibkrunProvider {
    async fn provision_inner(
        &self,
        id: VmId,
        spec: PreparedSpec,
    ) -> Result<Arc<dyn VmInstance>, VmError> {
        let runtime_dir = create_runtime_dir(&self.inner.config.runtime_root, &id.0)?;
        let cgroup = match Cgroup::create(&self.inner.config, &id.0) {
            Ok(cgroup) => cgroup,
            Err(error) => {
                let _cleanup_result = fs::remove_dir_all(&runtime_dir);
                return Err(error);
            }
        };

        info!(
            vm_id = %id.0,
            vcpus = spec.vcpus,
            memory_mib = spec.memory_mib,
            cpu_quota_micros = ?self.inner.config.limits.cpu_quota_micros,
            pids_max = self.inner.config.limits.pids_max,
            writable_disk_max_bytes = self.inner.config.limits.writable_disk_max_bytes,
            wall_clock_seconds = self.inner.config.limits.wall_clock_timeout.as_secs(),
            "provisioning VM resources"
        );
        let worker = match self
            .inner
            .worker_spawner
            .spawn(Arc::clone(&self.inner.config), spec, &runtime_dir, &cgroup)
            .await
        {
            Ok(worker) => worker,
            Err(error) => {
                let _cgroup_result = cgroup.cleanup();
                let _runtime_result = fs::remove_dir_all(&runtime_dir);
                return Err(error);
            }
        };

        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (terminal, _) = watch::channel(None);
        let (ready, _) = watch::channel(false);
        let (start_result, _) = watch::channel(None);
        let instance = Arc::new(LibkrunInstance {
            id,
            config: Arc::clone(&self.inner.config),
            worker,
            state: Mutex::new(Lifecycle::Provisioned),
            terminal,
            terminal_guard: Mutex::new(()),
            ready,
            start_result,
            events,
            resources: Mutex::new(Some(OwnedResources {
                runtime_dir,
                cgroup,
            })),
            provider_ids: Arc::clone(&self.inner),
        });
        instance.spawn_event_forwarder();
        instance.spawn_process_monitor();
        Ok(instance)
    }
}

struct LibkrunInstance {
    id: VmId,
    config: Arc<LibkrunConfig>,
    worker: Arc<dyn WorkerBackend>,
    state: Mutex<Lifecycle>,
    terminal: watch::Sender<Option<Terminal>>,
    terminal_guard: Mutex<()>,
    ready: watch::Sender<bool>,
    start_result: watch::Sender<Option<Result<(), ErrorSnapshot>>>,
    events: broadcast::Sender<VmEvent>,
    resources: Mutex<Option<OwnedResources>>,
    provider_ids: Arc<ProviderInner>,
}

#[derive(Clone)]
enum Terminal {
    Exited(VmExit),
    Destroyed,
}

enum Lifecycle {
    Provisioned,
    Starting,
    Running,
    Stopping,
    Exited,
    StartFailed(ErrorSnapshot),
    Destroyed,
}

struct OwnedResources {
    runtime_dir: PathBuf,
    cgroup: Cgroup,
}

#[async_trait]
impl VmInstance for LibkrunInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        let mut result_rx = self.start_result.subscribe();
        let leader = {
            let mut state = self.state.lock().await;
            let leader = match &*state {
                Lifecycle::Provisioned => {
                    *state = Lifecycle::Starting;
                    true
                }
                Lifecycle::Starting => false,
                Lifecycle::Running | Lifecycle::Stopping => return Ok(()),
                Lifecycle::Exited => {
                    return Err(VmError::InvalidState("an exited VM cannot be restarted"));
                }
                Lifecycle::StartFailed(error) => return Err(error.to_vm_error()),
                Lifecycle::Destroyed => return Err(VmError::Destroyed),
            };
            drop(state);
            leader
        };

        if !leader {
            return wait_start_result(&mut result_rx).await;
        }

        info!(vm_id = %self.id.0, "starting libkrun worker");
        let mut outcome = self.start_leader().await;
        let destroyed_during_start = {
            let mut state = self.state.lock().await;
            if matches!(*state, Lifecycle::Destroyed) {
                outcome = Err(VmError::Destroyed);
                true
            } else {
                match &outcome {
                    Ok(()) => *state = Lifecycle::Running,
                    Err(error) => {
                        *state = Lifecycle::StartFailed(ErrorSnapshot::from_vm_error(error));
                    }
                }
                false
            }
        };
        let snapshot = outcome.as_ref().err().map(ErrorSnapshot::from_vm_error);
        self.start_result
            .send_replace(Some(snapshot.map_or(Ok(()), Err)));
        if outcome.is_err() && !destroyed_during_start {
            let _cleanup_result = self.force_cleanup(true).await;
        }
        outcome
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        let should_stop = {
            let mut state = self.state.lock().await;
            match &*state {
                Lifecycle::Provisioned
                | Lifecycle::Exited
                | Lifecycle::StartFailed(_)
                | Lifecycle::Destroyed => return Ok(()),
                Lifecycle::Stopping => false,
                Lifecycle::Starting | Lifecycle::Running => {
                    *state = Lifecycle::Stopping;
                    true
                }
            }
        };
        if !should_stop {
            return self.wait_for_terminal().await.map(|_| ());
        }

        match mode {
            StopMode::Graceful { timeout: grace } => {
                let timeout_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX);
                let cancel = self
                    .worker
                    .request(WorkerCommand::Cancel { timeout_ms })
                    .await;
                if let Err(error) = cancel {
                    warn!(vm_id = %self.id.0, %error, "guest cancellation request failed");
                }
                if timeout(grace, self.wait_for_terminal()).await.is_err() {
                    warn!(vm_id = %self.id.0, "graceful stop timed out; killing worker");
                    self.worker.kill().await?;
                }
            }
            StopMode::Force => self.worker.kill().await?,
            _ => {
                return Err(VmError::Unsupported {
                    feature: "stop mode".to_owned(),
                    provider: PROVIDER_NAME.to_owned(),
                });
            }
        }
        self.wait_for_terminal().await.map(|_| ())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        self.wait_for_terminal().await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        let was_started = {
            let mut state = self.state.lock().await;
            let was_started = match &*state {
                Lifecycle::Destroyed => None,
                Lifecycle::Provisioned | Lifecycle::StartFailed(_) => Some(false),
                Lifecycle::Starting
                | Lifecycle::Running
                | Lifecycle::Stopping
                | Lifecycle::Exited => Some(true),
            };
            if was_started.is_some() {
                *state = Lifecycle::Destroyed;
            }
            was_started
        };

        match was_started {
            Some(was_started) => self.force_cleanup(was_started).await,
            None => self.cleanup_resources().await,
        }
    }
}

impl LibkrunInstance {
    async fn start_leader(&self) -> Result<(), VmError> {
        timeout(
            self.config.startup_timeout,
            self.worker.request(WorkerCommand::Start),
        )
        .await
        .map_err(|_| unavailable_error("worker startup", "startup request timed out"))??;

        let mut ready = self.ready.subscribe();
        let mut terminal = self.terminal.subscribe();
        timeout(self.config.readiness_timeout, async {
            loop {
                if *ready.borrow_and_update() {
                    return Ok(());
                }
                let current_terminal = terminal.borrow_and_update().clone();
                if let Some(outcome) = current_terminal {
                    return match outcome {
                        Terminal::Exited(exit) => Err(unavailable_error(
                            "guest readiness",
                            format!("guest exited before readiness: {exit:?}"),
                        )),
                        Terminal::Destroyed => Err(VmError::Destroyed),
                    };
                }
                tokio::select! {
                    result = ready.changed() => {
                        result.map_err(|error| provider_error("ready-channel", error))?;
                    }
                    result = terminal.changed() => {
                        result.map_err(|error| provider_error("terminal-channel", error))?;
                    }
                }
            }
        })
        .await
        .map_err(|_| unavailable_error("guest readiness", "readiness timeout elapsed"))?
    }

    async fn wait_for_terminal(&self) -> Result<VmExit, VmError> {
        let mut terminal = self.terminal.subscribe();
        loop {
            let current = terminal.borrow_and_update().clone();
            if let Some(outcome) = current {
                return match outcome {
                    Terminal::Exited(exit) => Ok(exit),
                    Terminal::Destroyed => Err(VmError::Destroyed),
                };
            }
            terminal
                .changed()
                .await
                .map_err(|error| provider_error("terminal-channel", error))?;
        }
    }

    fn spawn_event_forwarder(self: &Arc<Self>) {
        let instance = Arc::clone(self);
        let mut events = instance.worker.subscribe_events();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => instance.handle_worker_event(event).await,
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        warn!(vm_id = %instance.id.0, count, "worker event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    fn spawn_process_monitor(self: &Arc<Self>) {
        let instance = Arc::clone(self);
        let mut process_exit = instance.worker.subscribe_process_exit();
        tokio::spawn(async move {
            loop {
                let current = process_exit.borrow_and_update().clone();
                if let Some(status) = current {
                    instance.complete_exit(status.into_vm_exit()).await;
                    break;
                }
                if process_exit.changed().await.is_err() {
                    break;
                }
            }
        });

        let instance = Arc::clone(self);
        tokio::spawn(async move {
            let mut terminal = instance.terminal.subscribe();
            let completed = timeout(instance.config.limits.wall_clock_timeout, async {
                loop {
                    if terminal.borrow_and_update().is_some() {
                        break;
                    }
                    if terminal.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .is_ok();
            if !completed {
                warn!(vm_id = %instance.id.0, "wall-clock limit reached");
                let _kill_result = instance.worker.kill().await;
            }
        });
    }

    async fn handle_worker_event(&self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started {
                ingress,
                vmm_pid,
                passt_pid,
            } => {
                info!(
                    vm_id = %self.id.0,
                    vmm_pid,
                    ?passt_pid,
                    forwards = ingress.len(),
                    "microVM started"
                );
                send_event(
                    &self.events,
                    VmEvent::Started {
                        ingress: ingress.into_iter().map(to_port_forward).collect(),
                    },
                );
            }
            WorkerEvent::Ready => {
                self.ready.send_replace(true);
                info!(vm_id = %self.id.0, "guest ready");
                send_event(&self.events, VmEvent::Ready);
            }
            WorkerEvent::Log { stream, bytes } => {
                let stream = match stream {
                    WireLogStream::Stdout => LogStream::Stdout,
                    WireLogStream::Stderr => LogStream::Stderr,
                };
                send_event(&self.events, VmEvent::Log { stream, bytes });
            }
            WorkerEvent::Metric {
                name,
                value,
                labels,
            } => send_event(
                &self.events,
                VmEvent::Metric(VmMetric {
                    name,
                    value,
                    labels,
                }),
            ),
            WorkerEvent::Health { nonce } => {
                tracing::debug!(vm_id = %self.id.0, nonce, "guest health response");
            }
            WorkerEvent::FinalizeResult { message } => {
                send_event(&self.events, VmEvent::FinalizeResult { message });
            }
            WorkerEvent::Exited { code, signal } => {
                self.complete_exit(VmExit { code, signal }).await;
            }
            WorkerEvent::BackendFailure(failure) => {
                error!(
                    vm_id = %self.id.0,
                    code = %failure.code,
                    message = %failure.message,
                    "worker backend failure"
                );
            }
        }
    }

    async fn complete_exit(&self, exit: VmExit) {
        let _guard = self.terminal_guard.lock().await;
        if self.terminal.borrow().is_some() {
            return;
        }
        self.terminal
            .send_replace(Some(Terminal::Exited(exit.clone())));
        {
            let mut state = self.state.lock().await;
            if !matches!(*state, Lifecycle::Destroyed) {
                *state = Lifecycle::Exited;
            }
        }
        send_event(&self.events, VmEvent::Exited(exit));
    }

    async fn force_cleanup(&self, was_started: bool) -> Result<(), VmError> {
        if was_started {
            self.worker.kill().await?;
            let _status = timeout(self.config.startup_timeout, self.worker.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        } else {
            let guard = self.terminal_guard.lock().await;
            if self.terminal.borrow().is_none() {
                self.terminal.send_replace(Some(Terminal::Destroyed));
            }
            drop(guard);
            let _destroy_result = self.worker.request(WorkerCommand::Destroy).await;
            let _status = timeout(self.config.startup_timeout, self.worker.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        }

        self.cleanup_resources().await
    }

    async fn cleanup_resources(&self) -> Result<(), VmError> {
        let mut resources = self.resources.lock().await;
        if let Some(owned) = resources.as_ref() {
            cleanup_runtime(&owned.runtime_dir)?;
            owned.cgroup.cleanup()?;
            info!(
                vm_id = %self.id.0,
                runtime_dir = %owned.runtime_dir.display(),
                "VM resources cleaned"
            );
        }
        resources.take();
        drop(resources);
        self.provider_ids.ids.lock().await.remove(&self.id);
        Ok(())
    }
}

async fn wait_start_result(
    receiver: &mut watch::Receiver<Option<Result<(), ErrorSnapshot>>>,
) -> Result<(), VmError> {
    loop {
        let current = receiver.borrow_and_update().clone();
        if let Some(result) = current {
            return result.map_err(|error| error.to_vm_error());
        }
        receiver
            .changed()
            .await
            .map_err(|error| provider_error("start-channel", error))?;
    }
}

#[derive(Debug, Clone)]
struct ErrorSnapshot {
    kind: WireErrorKind,
    code: String,
    message: String,
}

impl ErrorSnapshot {
    fn from_vm_error(error: &VmError) -> Self {
        match error {
            VmError::InvalidSpec { field, reason } => Self {
                kind: WireErrorKind::InvalidSpec,
                code: field.clone(),
                message: reason.clone(),
            },
            VmError::Unsupported { feature, .. } => Self {
                kind: WireErrorKind::Unsupported,
                code: "unsupported".to_owned(),
                message: feature.clone(),
            },
            VmError::Unavailable { resource, reason } => Self {
                kind: WireErrorKind::Unavailable,
                code: resource.clone(),
                message: reason.clone(),
            },
            VmError::InvalidState(message) => Self {
                kind: WireErrorKind::InvalidState,
                code: "invalid-state".to_owned(),
                message: (*message).to_owned(),
            },
            VmError::Destroyed => Self {
                kind: WireErrorKind::Destroyed,
                code: "destroyed".to_owned(),
                message: error.to_string(),
            },
            VmError::Provider { code, .. } => Self {
                kind: WireErrorKind::Backend,
                code: code.clone(),
                message: error.to_string(),
            },
            _ => Self {
                kind: WireErrorKind::Backend,
                code: "start".to_owned(),
                message: error.to_string(),
            },
        }
    }

    fn to_vm_error(&self) -> VmError {
        wire_to_vm_error(WireError {
            kind: self.kind,
            code: self.code.clone(),
            message: self.message.clone(),
        })
    }
}

type PendingResponse = oneshot::Sender<Result<(), WireError>>;
type PendingRequests = Arc<Mutex<HashMap<u64, PendingResponse>>>;

struct WorkerClient {
    writer: Mutex<OwnedWriteHalf>,
    pending: PendingRequests,
    next_request: AtomicU64,
    events: broadcast::Sender<WorkerEvent>,
    child: Arc<Mutex<Child>>,
    process_exit: watch::Sender<Option<ProcessStatus>>,
    pid: u32,
}

#[async_trait]
trait WorkerBackend: Send + Sync {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError>;
    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent>;
    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>>;
    async fn kill(&self) -> Result<(), VmError>;
    async fn wait_process(&self) -> Result<ProcessStatus, VmError>;
}

#[async_trait]
trait WorkerSpawner: Send + Sync {
    async fn spawn(
        &self,
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
    ) -> Result<Arc<dyn WorkerBackend>, VmError>;
}

struct ProcessWorkerSpawner;

#[async_trait]
impl WorkerSpawner for ProcessWorkerSpawner {
    async fn spawn(
        &self,
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        WorkerClient::launch(config, spec, runtime_dir, cgroup)
            .await
            .map(|worker| worker as Arc<dyn WorkerBackend>)
    }
}

#[async_trait]
impl WorkerBackend for WorkerClient {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        Self::request(self, command).await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        Self::subscribe_events(self)
    }

    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        Self::subscribe_process_exit(self)
    }

    async fn kill(&self) -> Result<(), VmError> {
        Self::kill(self).await
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        Self::wait_process(self).await
    }
}

impl WorkerClient {
    async fn launch(
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
    ) -> Result<Arc<Self>, VmError> {
        let socket_path = runtime_dir.join("supervisor.sock");
        let listener = UnixListener::bind(&socket_path)
            .map_err(|error| provider_error("worker-listener", error))?;
        let mut command = Command::new(&config.worker_binary);
        command.arg("--socket").arg(&socket_path).kill_on_drop(true);
        let child = command
            .spawn()
            .map_err(|error| unavailable_error("worker binary", error.to_string()))?;
        let pid = child
            .id()
            .ok_or_else(|| unavailable_error("worker process", "worker has no PID"))?;
        if let Err(error) = cgroup.add_process(pid) {
            let mut child = child;
            let _kill_result = child.start_kill();
            let _wait_result = child.wait().await;
            return Err(error);
        }
        info!(
            worker_pid = pid,
            cgroup = %cgroup.path().display(),
            "worker placed in delegated cgroup"
        );

        let (stream, _) = timeout(config.startup_timeout, listener.accept())
            .await
            .map_err(|_| unavailable_error("worker IPC", "worker connection timed out"))?
            .map_err(|error| provider_error("worker-accept", error))?;
        let (reader, writer) = stream.into_split();
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (process_exit, _) = watch::channel(None);
        let client = Arc::new(Self {
            writer: Mutex::new(writer),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request: AtomicU64::new(1),
            events,
            child: Arc::new(Mutex::new(child)),
            process_exit,
            pid,
        });
        client.spawn_reader(reader);
        client.spawn_reaper();

        let worker_config = WorkerConfiguration {
            passt_binary: config.passt_binary.clone(),
            libkrun_library: config.libkrun_library.clone(),
            service_uid: config.service_uid,
            service_gid: config.service_gid,
            startup_timeout: config.startup_timeout,
            broker_socket_path: config.broker_socket_path.clone(),
        };
        if let Err(error) = client
            .request(WorkerCommand::Configure {
                config: worker_config,
                spec: Box::new(spec),
                runtime_dir: runtime_dir.to_path_buf(),
            })
            .await
        {
            let _kill_result = client.kill().await;
            let _wait_result = client.wait_process().await;
            return Err(error);
        }
        Ok(client)
    }

    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        let request_id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(request_id, sender);
        let write_result = write_async(
            &mut *self.writer.lock().await,
            &WorkerRequest {
                request_id,
                command,
            },
        )
        .await;
        if let Err(error) = write_result {
            self.pending.lock().await.remove(&request_id);
            return Err(provider_error("worker-write", error));
        }
        receiver
            .await
            .map_err(|error| provider_error("worker-response", error))?
            .map_err(wire_to_vm_error)
    }

    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        self.events.subscribe()
    }

    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.process_exit.subscribe()
    }

    async fn kill(&self) -> Result<(), VmError> {
        let mut child = self.child.lock().await;
        if child
            .try_wait()
            .map_err(|error| provider_error("worker-status", error))?
            .is_none()
        {
            child
                .start_kill()
                .map_err(|error| provider_error("worker-kill", error))?;
        }
        drop(child);
        Ok(())
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        let mut exit = self.process_exit.subscribe();
        loop {
            let current = exit.borrow_and_update().clone();
            if let Some(status) = current {
                return Ok(status);
            }
            exit.changed()
                .await
                .map_err(|error| provider_error("worker-exit-channel", error))?;
        }
    }

    fn spawn_reader(self: &Arc<Self>, mut reader: OwnedReadHalf) {
        let pending = Arc::clone(&self.pending);
        let events = self.events.clone();
        tokio::spawn(async move {
            loop {
                match read_async::<WorkerMessage>(&mut reader).await {
                    Ok(WorkerMessage::Response { request_id, result }) => {
                        let sender = pending.lock().await.remove(&request_id);
                        if let Some(sender) = sender {
                            let _send_result = sender.send(result);
                        }
                    }
                    Ok(WorkerMessage::Event(event)) => {
                        drop(events.send(event));
                    }
                    Err(error) => {
                        let failure = WireError {
                            kind: WireErrorKind::Unavailable,
                            code: "worker-ipc-closed".to_owned(),
                            message: error.to_string(),
                        };
                        let senders = pending
                            .lock()
                            .await
                            .drain()
                            .map(|(_, sender)| sender)
                            .collect::<Vec<_>>();
                        for sender in senders {
                            let _send_result = sender.send(Err(failure.clone()));
                        }
                        break;
                    }
                }
            }
        });
    }

    fn spawn_reaper(self: &Arc<Self>) {
        let child = Arc::clone(&self.child);
        let process_exit = self.process_exit.clone();
        let pid = self.pid;
        tokio::spawn(async move {
            loop {
                let status = {
                    let mut child = child.lock().await;
                    child.try_wait()
                };
                match status {
                    Ok(Some(status)) => {
                        let status = ProcessStatus::from(status);
                        info!(
                            worker_pid = pid,
                            code = ?status.code,
                            signal = ?status.signal,
                            "worker reaped"
                        );
                        process_exit.send_replace(Some(status));
                        break;
                    }
                    Ok(None) => sleep(PROCESS_POLL_INTERVAL).await,
                    Err(error) => {
                        error!(worker_pid = pid, %error, "failed to reap worker");
                        process_exit.send_replace(Some(ProcessStatus {
                            code: None,
                            signal: None,
                        }));
                        break;
                    }
                }
            }
        });
    }
}

#[derive(Debug, Clone)]
struct ProcessStatus {
    code: Option<i32>,
    signal: Option<i32>,
}

impl ProcessStatus {
    const fn into_vm_exit(self) -> VmExit {
        VmExit {
            code: self.code,
            signal: self.signal,
        }
    }
}

impl From<ExitStatus> for ProcessStatus {
    fn from(status: ExitStatus) -> Self {
        Self {
            code: status.code(),
            signal: status.signal(),
        }
    }
}

fn create_runtime_dir(root: &Path, id: &str) -> Result<PathBuf, VmError> {
    let path = root.join(id);
    fs::create_dir(&path).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => VmError::AlreadyExists(VmId(id.to_owned())),
        _ => provider_error("runtime-create", error),
    })?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .map_err(|error| provider_error("runtime-permissions", error))?;
    Ok(path)
}

fn cleanup_runtime(path: &Path) -> Result<(), VmError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(provider_error("runtime-cleanup", error)),
    }
}

const fn to_port_forward(forward: PreparedForward) -> PortForward {
    PortForward {
        protocol: PortProtocol::Tcp,
        bind_addr: forward.bind_addr,
        host_port: forward.host_port,
        guest_port: forward.guest_port,
    }
}

fn send_event(sender: &broadcast::Sender<VmEvent>, event: VmEvent) {
    drop(sender.send(event));
}

fn wire_to_vm_error(error: WireError) -> VmError {
    match error.kind {
        WireErrorKind::InvalidSpec => VmError::InvalidSpec {
            field: error.code,
            reason: error.message,
        },
        WireErrorKind::Unsupported => VmError::Unsupported {
            feature: error.message,
            provider: PROVIDER_NAME.to_owned(),
        },
        WireErrorKind::Unavailable => VmError::Unavailable {
            resource: error.code,
            reason: error.message,
        },
        WireErrorKind::InvalidState => VmError::InvalidState("worker rejected lifecycle operation"),
        WireErrorKind::Destroyed => VmError::Destroyed,
        WireErrorKind::Backend => VmError::Provider {
            provider: PROVIDER_NAME.to_owned(),
            code: error.code,
            source: Box::new(MessageError(error.message)),
        },
    }
}

fn unavailable_error(resource: impl Into<String>, reason: impl Into<String>) -> VmError {
    VmError::Unavailable {
        resource: resource.into(),
        reason: reason.into(),
    }
}

fn provider_error(code: &'static str, source: impl Error + Send + Sync + 'static) -> VmError {
    VmError::Provider {
        provider: PROVIDER_NAME.to_owned(),
        code: code.to_owned(),
        source: Box::new(source),
    }
}

#[derive(Debug)]
struct MessageError(String);

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MessageError {}

#[cfg(test)]
mod tests {
    use super::{
        ErrorSnapshot, LibkrunInstance, Lifecycle, ProcessStatus, ProcessWorkerSpawner,
        ProviderInner, Terminal, WorkerBackend, WorkerSpawner, create_runtime_dir,
        wire_to_vm_error,
    };
    use crate::{
        config::LibkrunConfig,
        worker::{WireError, WireErrorKind, WorkerCommand, WorkerEvent},
    };
    use async_trait::async_trait;
    use std::{
        collections::{BTreeMap, HashSet},
        fs,
        os::unix::fs::PermissionsExt as _,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tempfile::TempDir;
    use tokio::sync::{Mutex, Notify, broadcast, watch};
    use vm_trait::{
        DiskFormat, GuestCommand, NetworkMode, RootFilesystem, StopMode, VmDisk, VmError, VmEvent,
        VmId, VmInstance, VmProvider, VmResources, VmSpec,
    };

    #[test]
    fn runtime_directory_is_private_and_collision_is_typed() {
        let temp = TempDir::new().unwrap();
        let runtime = create_runtime_dir(temp.path(), "vm").unwrap();
        assert_eq!(
            fs::metadata(runtime).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(matches!(
            create_runtime_dir(temp.path(), "vm"),
            Err(VmError::AlreadyExists(_))
        ));
    }

    #[test]
    fn worker_errors_remain_typed() {
        let error = wire_to_vm_error(WireError {
            kind: WireErrorKind::Unavailable,
            code: "passt".to_owned(),
            message: "not installed".to_owned(),
        });
        assert!(matches!(
            error,
            VmError::Unavailable { resource, .. } if resource == "passt"
        ));
    }

    #[tokio::test]
    async fn failed_worker_launch_cleans_runtime_and_cgroup() {
        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let images = temp.path().join("images");
        let disks = temp.path().join("disks");
        let mounts = temp.path().join("mounts");
        let cgroups = temp.path().join("cgroups");
        for directory in [&runtime, &images, &disks, &mounts, &cgroups] {
            fs::create_dir(directory).unwrap();
        }
        let root = images.join("root");
        fs::create_dir(&root).unwrap();
        let caller_disk = disks.join("agent-state.raw");
        fs::write(&caller_disk, b"caller-owned-state").unwrap();
        let kvm = temp.path().join("kvm");
        fs::write(&kvm, "").unwrap();
        let mut config = LibkrunConfig::new(
            &runtime,
            vec![images],
            vec![disks],
            vec![mounts],
            "/bin/false",
            &cgroups,
        );
        config.passt_binary = PathBuf::from("/bin/false");
        config.kvm_device = kvm;
        config.enforce_cgroup_v2 = false;
        config.startup_timeout = Duration::from_millis(50);
        let provider = super::LibkrunProvider::new(config).unwrap();

        let mut requested = spec("worker-failure", root);
        requested.disks.push(VmDisk {
            id: String::from("instance-state"),
            host_path: caller_disk.clone(),
            format: DiskFormat::Raw,
            read_only: false,
        });
        let error = provider
            .provision(requested)
            .await
            .err()
            .expect("provision must fail");
        assert!(matches!(error, VmError::Unavailable { .. }));
        assert!(!runtime.join("worker-failure").exists());
        assert!(!cgroups.join("worker-failure").exists());
        assert_eq!(
            fs::read(&caller_disk).unwrap(),
            b"caller-owned-state",
            "failed provisioning modified caller-owned disk"
        );
    }

    #[tokio::test]
    async fn orphan_cleanup_preserves_caller_owned_backing() {
        let temp = TempDir::new().unwrap();
        let (config, _root, runtime, cgroups) = emulated_config(&temp);
        let disk = temp.path().join("disks").join("agent-state.raw");
        fs::write(&disk, b"persistent").unwrap();
        fs::create_dir(runtime.join("orphan")).unwrap();
        fs::write(runtime.join("orphan").join("socket"), []).unwrap();
        fs::create_dir(cgroups.join("orphan")).unwrap();
        let provider =
            super::LibkrunProvider::new_with_spawner(config, Arc::new(FailingSpawner)).unwrap();

        provider
            .cleanup_orphan(&VmId(String::from("orphan")))
            .await
            .unwrap();

        assert!(!runtime.join("orphan").exists());
        assert!(!cgroups.join("orphan").exists());
        assert_eq!(fs::read(disk).unwrap(), b"persistent");
    }

    #[tokio::test]
    async fn injected_spawner_failure_cleans_every_allocated_resource() {
        let temp = TempDir::new().unwrap();
        let (config, root, runtime, cgroups) = emulated_config(&temp);
        let provider =
            super::LibkrunProvider::new_with_spawner(config, Arc::new(FailingSpawner)).unwrap();
        let error = provider
            .provision(spec("spawner-failure", root))
            .await
            .err()
            .expect("injected worker spawn must fail");
        assert!(matches!(
            error,
            VmError::Unavailable { resource, .. } if resource == "worker spawn"
        ));
        assert!(!runtime.join("spawner-failure").exists());
        assert!(!cgroups.join("spawner-failure").exists());
    }

    #[tokio::test]
    async fn concurrent_start_is_shared_and_events_are_ordered() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::new());
        let instance = instance(&temp, worker.clone());
        let mut events = instance.subscribe_events();

        let first = Arc::clone(&instance);
        let second = Arc::clone(&instance);
        let (first, second) = tokio::join!(first.start(), second.start());
        first.unwrap();
        second.unwrap();
        assert_eq!(worker.start_calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            events.recv().await.unwrap(),
            VmEvent::Started { .. }
        ));
        assert!(matches!(events.recv().await.unwrap(), VmEvent::Ready));
    }

    #[tokio::test]
    async fn many_waiters_receive_one_cached_exit() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::new());
        let instance = instance(&temp, worker);
        instance.start().await.unwrap();

        let first = Arc::clone(&instance);
        let second = Arc::clone(&instance);
        let (first, second, stopped) = tokio::join!(
            first.wait(),
            second.wait(),
            instance.stop(StopMode::Graceful {
                timeout: Duration::from_secs(1),
            })
        );
        stopped.unwrap();
        assert_eq!(first.unwrap(), second.unwrap());
        assert_eq!(instance.wait().await.unwrap().code, Some(0));
    }

    #[tokio::test]
    async fn destroy_before_start_is_idempotent_and_typed() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::new());
        let instance = instance(&temp, worker);
        instance.destroy().await.unwrap();
        instance.destroy().await.unwrap();
        assert!(matches!(instance.wait().await, Err(VmError::Destroyed)));
    }

    #[tokio::test]
    async fn concurrent_start_callers_share_a_typed_failure() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::failing_start());
        let instance = instance(&temp, worker.clone());
        let first = Arc::clone(&instance);
        let second = Arc::clone(&instance);
        let (first, second) = tokio::join!(first.start(), second.start());
        assert!(matches!(first, Err(VmError::Unavailable { .. })));
        assert!(matches!(second, Err(VmError::Unavailable { .. })));
        assert_eq!(worker.start_calls.load(Ordering::Relaxed), 1);
        assert!(matches!(
            instance.start().await,
            Err(VmError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn destroy_during_startup_does_not_resurrect_instance() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::blocking_start());
        let instance = instance(&temp, worker.clone());
        let start_instance = Arc::clone(&instance);
        let started = tokio::spawn(async move { start_instance.start().await });
        worker.start_entered.notified().await;

        instance.destroy().await.unwrap();
        worker.release_start.notify_one();

        let start_result = started.await.unwrap();
        assert!(
            matches!(start_result, Err(VmError::Destroyed)),
            "start completed with {start_result:?}"
        );
        assert!(matches!(instance.start().await, Err(VmError::Destroyed)));
        instance.destroy().await.unwrap();
    }

    #[tokio::test]
    async fn worker_crash_without_exit_event_is_cached_and_forwarded_once() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::new());
        let instance = instance(&temp, worker.clone());
        let mut events = instance.subscribe_events();
        instance.start().await.unwrap();
        assert!(matches!(
            events.recv().await.unwrap(),
            VmEvent::Started { .. }
        ));
        assert!(matches!(events.recv().await.unwrap(), VmEvent::Ready));

        worker.crash(11);
        let exit = instance.wait().await.unwrap();
        assert_eq!(exit.signal, Some(11));
        match events.recv().await.unwrap() {
            VmEvent::Exited(event_exit) => assert_eq!(event_exit, exit),
            event => panic!("expected terminal event, received {event:?}"),
        }
        assert_eq!(instance.wait().await.unwrap(), exit);
        assert!(
            tokio::time::timeout(Duration::from_millis(25), events.recv())
                .await
                .is_err()
        );
        instance.destroy().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_timeout_is_typed_and_force_cleans_worker() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::without_ready());
        let instance = instance(&temp, worker);
        let starting_instance = Arc::clone(&instance);
        let starting = tokio::spawn(async move { starting_instance.start().await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(21)).await;
        assert!(matches!(
            starting.await.unwrap(),
            Err(VmError::Unavailable { resource, .. }) if resource == "guest readiness"
        ));
        assert!(instance.wait().await.is_ok());
    }

    #[tokio::test]
    async fn cached_exit_survives_lagged_log_subscriber() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::new());
        let instance = instance(&temp, worker.clone());
        let mut slow_events = instance.subscribe_events();
        instance.start().await.unwrap();
        for index in 0..64 {
            drop(worker.events.send(WorkerEvent::Log {
                stream: crate::worker::WireLogStream::Stdout,
                bytes: index.to_string().into_bytes(),
            }));
            tokio::task::yield_now().await;
        }
        worker.exit(Some(23), None);
        assert_eq!(instance.wait().await.unwrap().code, Some(23));
        assert!(matches!(
            slow_events.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
        assert_eq!(instance.wait().await.unwrap().code, Some(23));
    }

    #[tokio::test(start_paused = true)]
    async fn wall_clock_limit_terminates_running_worker() {
        let temp = TempDir::new().unwrap();
        let worker = Arc::new(MockWorker::new());
        let instance = instance_with_wall_clock(&temp, worker, Duration::from_millis(20));
        instance.start().await.unwrap();
        tokio::time::advance(Duration::from_millis(21)).await;
        let exit = instance.wait().await.unwrap();
        assert_eq!(exit.signal, Some(9));
    }

    fn instance(temp: &TempDir, worker: Arc<MockWorker>) -> Arc<LibkrunInstance> {
        instance_with_wall_clock(temp, worker, Duration::from_secs(60))
    }

    fn instance_with_wall_clock(
        temp: &TempDir,
        worker: Arc<MockWorker>,
        wall_clock_timeout: Duration,
    ) -> Arc<LibkrunInstance> {
        let mut config = LibkrunConfig::new(
            temp.path(),
            vec![temp.path().to_path_buf()],
            vec![temp.path().to_path_buf()],
            vec![temp.path().to_path_buf()],
            "/bin/true",
            temp.path(),
        );
        config.startup_timeout = Duration::from_millis(50);
        config.readiness_timeout = Duration::from_millis(20);
        config.limits.wall_clock_timeout = wall_clock_timeout;
        let config = Arc::new(config);
        let provider_ids = Arc::new(ProviderInner {
            config: Arc::clone(&config),
            ids: Mutex::new(HashSet::from([VmId("test".to_owned())])),
            worker_spawner: Arc::new(ProcessWorkerSpawner),
        });
        let (events, _) = broadcast::channel(32);
        let (terminal, _) = watch::channel(None::<Terminal>);
        let (ready, _) = watch::channel(false);
        let (start_result, _) = watch::channel(None::<Result<(), ErrorSnapshot>>);
        let instance = Arc::new(LibkrunInstance {
            id: VmId("test".to_owned()),
            config,
            worker,
            state: Mutex::new(Lifecycle::Provisioned),
            terminal,
            terminal_guard: Mutex::new(()),
            ready,
            start_result,
            events,
            resources: Mutex::new(None),
            provider_ids,
        });
        instance.spawn_event_forwarder();
        instance.spawn_process_monitor();
        instance
    }

    fn spec(id: &str, root: PathBuf) -> VmSpec {
        VmSpec {
            id: VmId(id.to_owned()),
            root: RootFilesystem::Directory { host_path: root },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 256,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: "/bin/true".to_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from("/")),
            },
            labels: BTreeMap::new(),
        }
    }

    fn emulated_config(temp: &TempDir) -> (LibkrunConfig, PathBuf, PathBuf, PathBuf) {
        let runtime = temp.path().join("runtime");
        let images = temp.path().join("images");
        let disks = temp.path().join("disks");
        let mounts = temp.path().join("mounts");
        let cgroups = temp.path().join("cgroups");
        for directory in [&runtime, &images, &disks, &mounts, &cgroups] {
            fs::create_dir(directory).unwrap();
        }
        let root = images.join("root");
        fs::create_dir(&root).unwrap();
        let kvm = temp.path().join("kvm");
        fs::write(&kvm, "").unwrap();
        let mut config = LibkrunConfig::new(
            &runtime,
            vec![images],
            vec![disks],
            vec![mounts],
            "/bin/true",
            &cgroups,
        );
        config.kvm_device = kvm;
        config.enforce_cgroup_v2 = false;
        (config, root, runtime, cgroups)
    }

    struct FailingSpawner;

    #[async_trait]
    impl WorkerSpawner for FailingSpawner {
        async fn spawn(
            &self,
            _config: Arc<LibkrunConfig>,
            _spec: crate::validation::PreparedSpec,
            _runtime_dir: &std::path::Path,
            _cgroup: &crate::cgroup::Cgroup,
        ) -> Result<Arc<dyn WorkerBackend>, VmError> {
            Err(VmError::Unavailable {
                resource: String::from("worker spawn"),
                reason: String::from("deliberate test failure"),
            })
        }
    }

    struct MockWorker {
        events: broadcast::Sender<WorkerEvent>,
        process_exit: watch::Sender<Option<ProcessStatus>>,
        start_calls: AtomicUsize,
        send_ready: bool,
        fail_start: bool,
        block_start: bool,
        start_entered: Notify,
        release_start: Notify,
    }

    impl MockWorker {
        fn new() -> Self {
            let (events, _) = broadcast::channel(32);
            let (process_exit, _) = watch::channel(None);
            Self {
                events,
                process_exit,
                start_calls: AtomicUsize::new(0),
                send_ready: true,
                fail_start: false,
                block_start: false,
                start_entered: Notify::new(),
                release_start: Notify::new(),
            }
        }

        fn without_ready() -> Self {
            Self {
                send_ready: false,
                ..Self::new()
            }
        }

        fn failing_start() -> Self {
            Self {
                fail_start: true,
                ..Self::new()
            }
        }

        fn blocking_start() -> Self {
            Self {
                block_start: true,
                ..Self::new()
            }
        }

        fn exit(&self, code: Option<i32>, signal: Option<i32>) {
            drop(self.events.send(WorkerEvent::Exited { code, signal }));
            self.process_exit
                .send_replace(Some(ProcessStatus { code, signal }));
        }

        fn crash(&self, signal: i32) {
            self.process_exit.send_replace(Some(ProcessStatus {
                code: None,
                signal: Some(signal),
            }));
        }
    }

    #[async_trait]
    impl WorkerBackend for MockWorker {
        async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
            match command {
                WorkerCommand::Start => {
                    self.start_calls.fetch_add(1, Ordering::Relaxed);
                    if self.block_start {
                        self.start_entered.notify_one();
                        self.release_start.notified().await;
                    }
                    if self.fail_start {
                        return Err(VmError::Unavailable {
                            resource: String::from("mock start"),
                            reason: String::from("deliberate failure"),
                        });
                    }
                    drop(self.events.send(WorkerEvent::Started {
                        ingress: Vec::new(),
                        vmm_pid: 1,
                        passt_pid: None,
                    }));
                    if self.send_ready {
                        drop(self.events.send(WorkerEvent::Ready));
                    }
                }
                WorkerCommand::Cancel { .. } => self.exit(Some(0), None),
                WorkerCommand::Destroy => {
                    self.process_exit.send_replace(Some(ProcessStatus {
                        code: Some(0),
                        signal: None,
                    }));
                }
                WorkerCommand::Configure { .. } | WorkerCommand::Health { .. } => {}
            }
            Ok(())
        }

        fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
            self.events.subscribe()
        }

        fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
            self.process_exit.subscribe()
        }

        async fn kill(&self) -> Result<(), VmError> {
            self.exit(None, Some(9));
            Ok(())
        }

        async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
            let mut exit = self.process_exit.subscribe();
            loop {
                let current = exit.borrow_and_update().clone();
                if let Some(status) = current {
                    return Ok(status);
                }
                exit.changed().await.unwrap();
            }
        }
    }
}
