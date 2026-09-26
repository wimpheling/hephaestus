use super::*;
use async_trait::async_trait;
pub(super) struct ServiceReadyVm {
    pub(super) id: VmId,
    pub(super) events: tokio::sync::broadcast::Sender<VmEvent>,
    pub(super) fail_destroy: Arc<AtomicBool>,
    pub(super) destroy_gate: Arc<Mutex<Option<Arc<Notify>>>>,
    pub(super) destroy_started: Arc<Mutex<Option<Arc<Notify>>>>,
    pub(super) destroy_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl VmInstance for ServiceReadyVm {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        Ok(())
    }

    async fn stop(&self, _: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        std::future::pending().await
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        let (client, mut peer) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut request = [0_u8; 2048];
            let _ = peer.read(&mut request).await;
            peer.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .await
                .expect("readiness response");
        });
        Ok(Box::new(client))
    }

    fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        self.destroy_calls.fetch_add(1, Ordering::Relaxed);
        if self.fail_destroy.load(Ordering::Relaxed) {
            Err(VmError::Unavailable {
                resource: String::from("test VM"),
                reason: String::from("deliberate cleanup failure"),
            })
        } else {
            if let Some(started) = self
                .destroy_started
                .lock()
                .expect("destroy started")
                .as_ref()
            {
                started.notify_one();
            }
            let gate = self.destroy_gate.lock().expect("destroy gate").clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            Ok(())
        }
    }
}

pub(super) struct ReadyProvider {
    pub(super) fail_destroy: Arc<AtomicBool>,
    pub(super) destroy_gate: Arc<Mutex<Option<Arc<Notify>>>>,
    pub(super) destroy_started: Arc<Mutex<Option<Arc<Notify>>>>,
    pub(super) destroy_calls: Arc<AtomicUsize>,
    pub(super) orphan_cleanup_calls: AtomicUsize,
    pub(super) last_vm: Mutex<Option<Arc<dyn VmInstance>>>,
}

#[async_trait]
impl VmProvider for ReadyProvider {
    fn name(&self) -> &'static str {
        "supervisor-ready-test"
    }

    async fn provision(&self, spec: vm_trait::VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let (events, _) = tokio::sync::broadcast::channel(8);
        let vm: Arc<dyn VmInstance> = Arc::new(ServiceReadyVm {
            id: spec.id,
            events,
            fail_destroy: Arc::clone(&self.fail_destroy),
            destroy_gate: Arc::clone(&self.destroy_gate),
            destroy_started: Arc::clone(&self.destroy_started),
            destroy_calls: Arc::clone(&self.destroy_calls),
        });
        *self.last_vm.lock().expect("last VM") = Some(Arc::clone(&vm));
        Ok(vm)
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        let _ = id;
        self.orphan_cleanup_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

pub(super) struct ReadyResolver {
    pub(super) launch: crate::GatewayServiceLaunch,
}

#[async_trait]
impl GatewayServiceLaunchResolver for ReadyResolver {
    async fn resolve_service_launch(
        &self,
        _: crate::GatewayServiceLaunchRequest,
    ) -> Result<crate::GatewayServiceLaunch, GatewayEdgeError> {
        Ok(self.launch.clone())
    }

    async fn cleanup_service_launch(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        Ok(())
    }
}

pub(super) struct ReadyFailureStore;

#[async_trait]
impl GatewayServiceFailureStore for ReadyFailureStore {
    async fn record_failure(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError> {
        Ok(())
    }
}
