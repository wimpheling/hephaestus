use super::*;

pub(in crate::provider::tests) struct FailingSpawner;

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

pub(in crate::provider::tests) struct MockWorker {
    pub(in crate::provider::tests) events: broadcast::Sender<WorkerEvent>,
    pub(in crate::provider::tests) service_requests:
        broadcast::Sender<PrivateServiceConnectionMessage>,
    pub(in crate::provider::tests) process_exit: watch::Sender<Option<ProcessStatus>>,
    pub(in crate::provider::tests) start_calls: AtomicUsize,
    pub(in crate::provider::tests) send_ready: bool,
    pub(in crate::provider::tests) fail_start: bool,
    pub(in crate::provider::tests) block_start: bool,
    pub(in crate::provider::tests) block_service: std::sync::atomic::AtomicBool,
    pub(in crate::provider::tests) start_entered: Notify,
    pub(in crate::provider::tests) release_start: Notify,
    pub(in crate::provider::tests) service_entered: Notify,
    pub(in crate::provider::tests) release_service: Notify,
    pub(in crate::provider::tests) service_finished: Notify,
}

impl MockWorker {
    pub(in crate::provider::tests) fn new() -> Self {
        let (events, _) = broadcast::channel(32);
        let (service_requests, _) = broadcast::channel(8);
        let (process_exit, _) = watch::channel(None);
        Self {
            events,
            service_requests,
            process_exit,
            start_calls: AtomicUsize::new(0),
            send_ready: true,
            fail_start: false,
            block_start: false,
            block_service: std::sync::atomic::AtomicBool::new(false),
            start_entered: Notify::new(),
            release_start: Notify::new(),
            service_entered: Notify::new(),
            release_service: Notify::new(),
            service_finished: Notify::new(),
        }
    }

    pub(in crate::provider::tests) fn without_ready() -> Self {
        Self {
            send_ready: false,
            ..Self::new()
        }
    }

    pub(in crate::provider::tests) fn failing_start() -> Self {
        Self {
            fail_start: true,
            ..Self::new()
        }
    }

    pub(in crate::provider::tests) fn blocking_start() -> Self {
        Self {
            block_start: true,
            ..Self::new()
        }
    }

    pub(in crate::provider::tests) fn blocking_service() -> Self {
        let worker = Self::new();
        worker.block_service.store(true, Ordering::Relaxed);
        worker
    }

    pub(in crate::provider::tests) fn exit(&self, code: Option<i32>, signal: Option<i32>) {
        drop(self.events.send(WorkerEvent::Exited { code, signal }));
        self.process_exit
            .send_replace(Some(ProcessStatus { code, signal }));
    }

    pub(in crate::provider::tests) fn crash(&self, signal: i32) {
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
            WorkerCommand::OpenPrivateServiceConnection { connection } => {
                if self.block_service.load(Ordering::Relaxed) {
                    self.service_entered.notify_one();
                    self.release_service.notified().await;
                }
                drop(self.service_requests.send(connection));
                self.service_finished.notify_one();
            }
            WorkerCommand::InvokePrivateHttp { request_id, .. } => {
                drop(self.events.send(WorkerEvent::PrivateHttpResponse {
                    request_id,
                    response: PrivateHttpResponseMessage {
                        status: 201,
                        headers: Vec::new(),
                        body: b"response".to_vec(),
                        mailbox_publication: None,
                    },
                }));
            }
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
