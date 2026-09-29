use super::common::{
    Arc, AtomicU64, Cgroup, Command, EVENT_CAPACITY, HashMap, LibkrunConfig, Mutex, Ordering,
    OwnedReadHalf, PROCESS_POLL_INTERVAL, Path, PreparedSpec, SUPERVISOR_SOCKET_NAME, VmError,
    WireError, WireErrorKind, WorkerClient, WorkerCommand, WorkerConfiguration, WorkerEvent,
    WorkerMessage, WorkerRequest, broadcast, error, info, oneshot, read_async, sleep, timeout,
    watch, write_async,
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

    pub(super) async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
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
