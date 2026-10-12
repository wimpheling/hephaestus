use super::common::{
    Arc, AtomicU64, Cgroup, Command, EVENT_CAPACITY, ErrorSnapshot, ExitStatus, HashMap,
    LibkrunConfig, Mutex, Ordering, OwnedReadHalf, PROCESS_POLL_INTERVAL, Path, PreparedSpec,
    SUPERVISOR_SOCKET_NAME, VmError, WireError, WireErrorKind, WorkerClient, WorkerCommand,
    WorkerConfiguration, WorkerEvent, WorkerMessage, WorkerRequest, broadcast, error, info,
    oneshot, read_async, sleep, timeout, watch, write_async,
};
use super::helpers::{ProcessStatus, provider_error, unavailable_error, wire_to_vm_error};
use tokio::net::UnixListener;

impl WorkerClient {
    pub(super) async fn launch(
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
    ) -> Result<Arc<Self>, VmError> {
        let socket_path = runtime_dir.join(SUPERVISOR_SOCKET_NAME);
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
            process_error: watch::channel(None).0,
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
            runtime_git_socket_path: config.runtime_git_socket_path.clone(),
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
        self.process_exit.subscribe()
    }

    pub(super) async fn kill(&self) -> Result<(), VmError> {
        let mut child = self.child.lock().await;
        if publish_process_poll(child.try_wait(), &self.process_exit, &self.process_error)?
            .is_none()
        {
            child
                .start_kill()
                .map_err(|error| provider_error("worker-kill", error))?;
        }
        drop(child);
        Ok(())
    }

    pub(super) async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        let mut exit = self.process_exit.subscribe();
        let mut errors = self.process_error.subscribe();
        // Retry the retained Child, rather than accepting a past error as exit
        // or requiring a dead reaper task to manufacture a notification.
        let process_status = {
            let mut child = self.child.lock().await;
            let result =
                publish_process_poll(child.try_wait(), &self.process_exit, &self.process_error);
            drop(child);
            result
        };
        if let Some(status) = process_status? {
            return Ok(status);
        }
        loop {
            let status = exit.borrow_and_update().clone();
            if let Some(status) = status {
                return Ok(status);
            }
            let error = errors.borrow_and_update().clone();
            if let Some(error) = error {
                return Err(error.to_vm_error());
            }
            tokio::select! {
                result = exit.changed() => {
                    result.map_err(|error| provider_error("worker-exit-channel", error))?;
                }
                result = errors.changed() => {
                    result.map_err(|error| provider_error("worker-reap-channel", error))?;
                }
            }
        }
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

    pub(super) fn spawn_reaper(self: &Arc<Self>) {
        let child = Arc::clone(&self.child);
        let process_exit = self.process_exit.clone();
        let process_error = self.process_error.clone();
        let pid = self.pid;
        tokio::spawn(async move {
            let mut last_error = None;
            loop {
                let status = {
                    let mut child = child.lock().await;
                    publish_process_poll(child.try_wait(), &process_exit, &process_error)
                };
                match status {
                    Ok(Some(status)) => {
                        info!(
                            worker_pid = pid,
                            code = ?status.code,
                            signal = ?status.signal,
                            "worker reaped"
                        );
                        break;
                    }
                    Ok(None) => {
                        last_error = None;
                        sleep(PROCESS_POLL_INTERVAL).await;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if last_error.as_ref() != Some(&message) {
                            error!(worker_pid = pid, %error, "failed to reap worker");
                            last_error = Some(message);
                        }
                        // The Child stays retained and is polled again; an
                        // error notification never becomes an exit observation.
                        sleep(PROCESS_POLL_INTERVAL).await;
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
