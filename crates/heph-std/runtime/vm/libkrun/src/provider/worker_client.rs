use super::common::{
    Arc, AtomicU64, Cgroup, Command, EVENT_CAPACITY, ErrorSnapshot, ExitStatus, HashMap,
    LaunchCustody, LibkrunConfig, Mutex, Ordering, OwnedReadHalf, Path, PreparedSpec,
    SUPERVISOR_SOCKET_NAME, VmError, WireError, WireErrorKind, WorkerClient, WorkerCommand,
    WorkerConfiguration, WorkerEvent, WorkerMessage, WorkerRequest, broadcast, info, oneshot,
    read_async, timeout, watch, write_async,
};
use super::helpers::{ProcessStatus, provider_error, unavailable_error, wire_to_vm_error};
use tokio::net::UnixListener;

impl WorkerClient {
    pub(super) async fn launch(
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
        custody: &LaunchCustody,
    ) -> Result<Arc<Self>, VmError> {
        Self::launch_with_after_spawn(config, spec, runtime_dir, cgroup, custody, |_| Ok(())).await
    }

    pub(super) async fn launch_with_after_spawn(
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
        custody: &LaunchCustody,
        after_spawn: impl FnOnce(&Arc<super::worker_process::WorkerProcess>) -> Result<(), VmError>,
    ) -> Result<Arc<Self>, VmError> {
        let runtime = custody.runtime_file()?;
        let host_socket = custody.host_runtime_dir()?.join(SUPERVISOR_SOCKET_NAME);
        let listener = UnixListener::bind(&host_socket)
            .map_err(|error| provider_error("worker-listener", error))?;
        let mut command = Command::new(&config.worker_binary);
        // Stdio performs a child-local dup to FD0 without changing CLOEXEC in
        // the supervisor or inheriting any ownership lock. IPC uses its socket.
        command
            .arg("--runtime-stdin")
            .stdin(super::containment::worker_stdin(&runtime)?)
            .kill_on_drop(true);
        // Spawn and publish the retained Child synchronously before placement
        // or the first IPC await. Launch's owning task handles every failure.
        let process = custody.spawn(&mut command);
        // Command/Stdio own the last parent gate duplicate. Drop, never LOCK_UN:
        // unlocking its OFD would also unlock the child's inherited description.
        drop(command);
        let process = process?;
        after_spawn(&process)?;
        let pid = process
            .pid
            .ok_or_else(|| unavailable_error("worker process", "worker has no PID"))?;
        cgroup.add_process(pid)?;
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
        let client = Arc::new(Self {
            writer: Mutex::new(writer),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request: AtomicU64::new(1),
            events,
            process,
        });
        client.spawn_reader(reader);
        custody.record_backend(client.clone())?;

        let worker_config = WorkerConfiguration {
            passt_binary: config.passt_binary.clone(),
            libkrun_library: config.libkrun_library.clone(),
            service_uid: config.service_uid,
            service_gid: config.service_gid,
            startup_timeout: config.startup_timeout,
            broker_socket_path: config.broker_socket_path.clone(),
            runtime_git_socket_path: config.runtime_git_socket_path.clone(),
        };
        timeout(
            config.startup_timeout,
            client.request(WorkerCommand::Configure {
                config: worker_config,
                spec: Box::new(spec),
                // Legacy wire field; inherited-mode worker resolves only its
                // retained child-local descriptor, never this configured path.
                runtime_dir: runtime_dir.to_path_buf(),
            }),
        )
        .await
        .map_err(|_| {
            unavailable_error("worker configuration", "configuration request timed out")
        })??;
        Ok(client)
    }

    pub(super) async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
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

    pub(super) fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        self.events.subscribe()
    }

    pub(super) fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.process.subscribe_process_exit()
    }

    pub(super) async fn kill(&self) -> Result<(), VmError> {
        self.process.kill().await
    }

    pub(super) async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.process.wait_process().await
    }

    pub(super) fn spawn_reader(self: &Arc<Self>, mut reader: OwnedReadHalf) {
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
}

// This is the sole publication boundary for real Child::try_wait results.
// Err retains unresolved process state; only Ok(Some) confirms actual reap.
pub(super) fn publish_process_poll(
    result: std::io::Result<Option<ExitStatus>>,
    exit: &watch::Sender<Option<ProcessStatus>>,
    errors: &watch::Sender<Option<ErrorSnapshot>>,
) -> Result<Option<ProcessStatus>, VmError> {
    match result {
        Ok(Some(status)) => {
            let status = ProcessStatus::from(status);
            exit.send_replace(Some(status.clone()));
            errors.send_replace(None);
            Ok(Some(status))
        }
        Ok(None) => {
            errors.send_replace(None);
            Ok(None)
        }
        Err(error) => {
            let error = provider_error("worker-reap", error);
            errors.send_replace(Some(ErrorSnapshot::from_vm_error(&error)));
            Err(error)
        }
    }
}
