use super::*;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub owner: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub route: Uuid,
    pub service_instance: Option<Uuid>,
}

pub type DebugServiceInstanceRow = (Uuid, String, i64, Option<String>, Option<i32>);

pub struct RecoveryProvider {
    pub reconciles: Arc<AtomicUsize>,
}

pub struct BlockingCaddyProvider {
    pub started: Arc<tokio::sync::Notify>,
    pub release: Arc<tokio::sync::Notify>,
    pub reconciles: Arc<AtomicUsize>,
}

impl RecoveryProvider {
    pub fn new(reconciles: Arc<AtomicUsize>) -> Self {
        Self { reconciles }
    }
}

#[derive(Clone)]
pub struct ServiceTransportProvider {
    pub inner: FakeProvider,
    pub provisioned: Arc<AtomicUsize>,
    pub destroyed: Arc<AtomicUsize>,
    pub destroy_gate: Option<Arc<DestroyGate>>,
    pub event_sender: Arc<Mutex<Option<tokio::sync::broadcast::Sender<VmEvent>>>>,
    pub inject_events: bool,
}

#[derive(Clone)]
pub struct DestroyGate {
    pub entered: Arc<tokio::sync::Notify>,
    pub release: Arc<tokio::sync::Notify>,
    pub block_once: Arc<std::sync::atomic::AtomicBool>,
    pub fail_once: Arc<std::sync::atomic::AtomicBool>,
    pub block_retry: Arc<std::sync::atomic::AtomicBool>,
    pub hold_after_first_failure: Arc<std::sync::atomic::AtomicBool>,
    pub first_failure_release: Arc<tokio::sync::Notify>,
    pub renewal_pause_entered: Arc<tokio::sync::Notify>,
    pub renewal_pause_release: Arc<tokio::sync::Notify>,
    pub renewal_pause_armed: Arc<std::sync::atomic::AtomicBool>,
    pub retry_target_id: Arc<Mutex<Option<VmId>>>,
    pub attempts: Arc<AtomicUsize>,
    pub attempt_ids: Arc<Mutex<Vec<VmId>>>,
    pub orphan_attempt_ids: Arc<Mutex<Vec<VmId>>>,
}

impl DestroyGate {
    pub fn new() -> Self {
        Self {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
            block_once: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            fail_once: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            block_retry: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            hold_after_first_failure: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            first_failure_release: Arc::new(tokio::sync::Notify::new()),
            renewal_pause_entered: Arc::new(tokio::sync::Notify::new()),
            renewal_pause_release: Arc::new(tokio::sync::Notify::new()),
            renewal_pause_armed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            retry_target_id: Arc::new(Mutex::new(None)),
            attempts: Arc::new(AtomicUsize::new(0)),
            attempt_ids: Arc::new(Mutex::new(Vec::new())),
            orphan_attempt_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn arm_first_failure_for(&self, vm_id: VmId) {
        *self.retry_target_id.lock().expect("retry target id") = Some(vm_id);
        self.fail_once.store(true, Ordering::Release);
        self.block_once.store(false, Ordering::Release);
        self.block_retry.store(true, Ordering::Release);
    }

    pub fn hold_first_failure(&self) {
        self.hold_after_first_failure.store(true, Ordering::Release);
    }

    pub fn release_first_failure(&self) {
        self.first_failure_release.notify_one();
    }

    pub fn arm_renewal_pause(&self) {
        self.renewal_pause_armed.store(true, Ordering::Release);
    }

    pub fn renewal_pause_entered(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.renewal_pause_entered)
    }

    pub fn release_renewal_pause(&self) {
        self.renewal_pause_armed.store(false, Ordering::Release);
        self.renewal_pause_release.notify_one();
    }

    pub async fn pause_renewal_if_armed(&self, vm_id: &str) {
        let is_retry_target = self
            .retry_target_id
            .lock()
            .expect("retry target id")
            .as_ref()
            .is_some_and(|target| target.0 == vm_id);
        if is_retry_target && self.renewal_pause_armed.load(Ordering::Acquire) {
            self.renewal_pause_entered.notify_one();
            self.renewal_pause_release.notified().await;
        }
    }
}

pub struct ServiceTransportVm {
    pub inner: Arc<dyn VmInstance>,
    pub destroyed: Arc<AtomicUsize>,
    pub destroy_gate: Option<Arc<DestroyGate>>,
    pub events: Option<tokio::sync::broadcast::Sender<VmEvent>>,
}

#[async_trait]
impl VmProvider for ServiceTransportProvider {
    fn name(&self) -> &'static str {
        "fake-service-transport"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provisioned.fetch_add(1, Ordering::AcqRel);
        let inner = self.inner.provision(spec).await?;
        let events = if self.inject_events {
            let (events, _) = tokio::sync::broadcast::channel(64);
            *self.event_sender.lock().expect("service event sender") = Some(events.clone());
            Some(events)
        } else {
            None
        };
        Ok(Arc::new(ServiceTransportVm {
            inner,
            destroyed: Arc::clone(&self.destroyed),
            destroy_gate: self.destroy_gate.clone(),
            events,
        }))
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        if let Some(gate) = &self.destroy_gate {
            gate.orphan_attempt_ids
                .lock()
                .expect("orphan attempt ids")
                .push(id.clone());
        }
        self.inner.cleanup_orphan(id).await
    }
}

#[async_trait]
impl VmInstance for ServiceTransportVm {
    fn id(&self) -> &VmId {
        self.inner.id()
    }

    async fn start(&self) -> Result<(), VmError> {
        let result = self.inner.start().await;
        if let (Ok(()), Some(events)) = (&result, &self.events) {
            let _ = events.send(VmEvent::Started {
                ingress: Vec::new(),
            });
            let _ = events.send(VmEvent::Ready);
        }
        result
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        let result = self.inner.stop(mode).await;
        if let (Ok(()), Some(events)) = (&result, &self.events) {
            let _ = events.send(VmEvent::Log {
                stream: vm_trait::LogStream::Stderr,
                bytes: b"application-final-event".to_vec(),
            });
        }
        result
    }

    async fn wait(&self) -> Result<vm_trait::VmExit, VmError> {
        self.inner.wait().await
    }

    async fn invoke_private_http(
        &self,
        request: PrivateHttpRequest,
    ) -> Result<PrivateHttpResponse, VmError> {
        self.inner.invoke_private_http(request).await
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 512];
            loop {
                let count = tokio::io::AsyncReadExt::read(&mut server, &mut buffer).await?;
                if count == 0 {
                    return Ok::<(), std::io::Error>(());
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    tokio::io::AsyncWriteExt::write_all(
                        &mut server,
                        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await?;
                    request.clear();
                }
            }
        });
        Ok(Box::new(client))
    }

    fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VmEvent> {
        self.events.as_ref().map_or_else(
            || self.inner.subscribe_events(),
            tokio::sync::broadcast::Sender::subscribe,
        )
    }

    async fn destroy(&self) -> Result<(), VmError> {
        if let Some(gate) = &self.destroy_gate {
            let is_retry_target = gate
                .retry_target_id
                .lock()
                .expect("retry target id")
                .as_ref()
                .is_some_and(|target| target == self.id());
            gate.attempts.fetch_add(1, Ordering::AcqRel);
            gate.attempt_ids
                .lock()
                .expect("destroy attempt ids")
                .push(self.id().clone());
            if is_retry_target && gate.fail_once.swap(false, Ordering::AcqRel) {
                gate.entered.notify_one();
                if gate.hold_after_first_failure.load(Ordering::Acquire) {
                    gate.arm_renewal_pause();
                    gate.first_failure_release.notified().await;
                }
                return Err(VmError::Unavailable {
                    resource: String::from("test-destroy"),
                    reason: String::from("injected first-attempt failure"),
                });
            }
            if is_retry_target && gate.block_retry.swap(false, Ordering::AcqRel) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        if let Some(gate) = &self.destroy_gate
            && gate.block_once.swap(false, Ordering::AcqRel)
        {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        let result = self.inner.destroy().await;
        if result.is_ok() {
            self.destroyed.fetch_add(1, Ordering::Release);
        }
        result
    }
}
