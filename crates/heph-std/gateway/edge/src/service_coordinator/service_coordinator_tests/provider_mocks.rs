use super::*;

pub(super) struct MockResolver {
    pub(super) launch: GatewayServiceLaunch,
    pub(super) started: Notify,
    pub(super) release: Notify,
    pub(super) cleanups: AtomicUsize,
}

#[async_trait]
impl GatewayServiceLaunchResolver for MockResolver {
    async fn resolve_service_launch(
        &self,
        _: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(self.launch.clone())
    }

    async fn cleanup_service_launch(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

pub(super) struct MockProvider {
    pub(super) provisions: AtomicUsize,
    pub(super) vm: Option<Arc<MockVm>>,
    pub(super) started: Notify,
    pub(super) release: Notify,
    pub(super) blocked: AtomicBool,
}

#[async_trait]
impl VmProvider for MockProvider {
    fn name(&self) -> &'static str {
        "coordinator-test"
    }

    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provisions.fetch_add(1, Ordering::Relaxed);
        self.started.notify_one();
        if self.blocked.load(Ordering::Relaxed) {
            self.release.notified().await;
        }
        self.vm
            .clone()
            .map(|vm| vm as Arc<dyn VmInstance>)
            .ok_or_else(|| VmError::Unavailable {
                resource: String::from("test provider"),
                reason: String::from("provision must not be reached"),
            })
    }

    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

pub(super) struct MockVm {
    pub(super) id: VmId,
    pub(super) events: broadcast::Sender<VmEvent>,
    pub(super) connections: Mutex<VecDeque<vm_trait::BoxedPrivateServiceConnection>>,
    pub(super) starts: AtomicUsize,
    pub(super) destroys: AtomicUsize,
    pub(super) destroy_started: Notify,
    pub(super) destroy_release: Notify,
    pub(super) destroy_blocked: AtomicBool,
    pub(super) destroy_fails: AtomicBool,
    pub(super) wait_exit: Notify,
    pub(super) should_exit: AtomicBool,
    pub(super) opens: AtomicUsize,
    pub(super) health_activity: Option<Arc<Notify>>,
}

#[async_trait]
impl VmInstance for MockVm {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn stop(&self, _: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        self.wait_exit.notified().await;
        if self.should_exit.load(Ordering::Relaxed) {
            Ok(VmExit {
                code: Some(1),
                signal: None,
            })
        } else {
            future::pending().await
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<vm_trait::BoxedPrivateServiceConnection, VmError> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        if let Some(activity) = &self.health_activity {
            activity.notify_one();
        }
        self.connections
            .lock()
            .expect("connection mutex")
            .pop_front()
            .ok_or(VmError::InvalidState("no test connection"))
    }

    async fn destroy(&self) -> Result<(), VmError> {
        self.destroy_started.notify_one();
        if self.destroy_blocked.load(Ordering::Relaxed) {
            self.destroy_release.notified().await;
        }
        self.destroys.fetch_add(1, Ordering::Relaxed);
        if self.destroy_fails.load(Ordering::Relaxed) {
            Err(VmError::Unavailable {
                resource: String::from("test VM"),
                reason: String::from("destroy failed"),
            })
        } else {
            Ok(())
        }
    }
}
