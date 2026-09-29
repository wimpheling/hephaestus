use super::*;

pub(super) struct MockCountState {
    pub(super) responses: Mutex<VecDeque<Result<u64, GatewayEdgeError>>>,
    pub(super) started: Notify,
    pub(super) release: Notify,
    pub(super) blocked: AtomicBool,
}

pub(super) fn default_count_state() -> Arc<MockCountState> {
    Arc::new(MockCountState {
        responses: Mutex::new(VecDeque::new()),
        started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
    })
}

pub(super) struct MockTargets {
    pub(super) target: Option<crate::GatewayServiceOwnedTarget>,
    pub(super) count: Arc<MockCountState>,
}

#[async_trait]
impl GatewayServiceTargetStore for MockTargets {
    async fn list_service_targets(
        &self,
        _: crate::GatewayServiceTargetPage,
    ) -> Result<crate::GatewayServiceTargetPageResult, GatewayEdgeError> {
        Ok(crate::GatewayServiceTargetPageResult {
            targets: Vec::new(),
            next_after: None,
        })
    }

    async fn get_service_target(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
        Ok(self.target.clone())
    }

    async fn get_service_instance(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        Ok(None)
    }

    async fn count_accepted_service_invocations(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        Ok(0)
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        _: GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        self.count.started.notify_one();
        if self.count.blocked.load(Ordering::Relaxed) {
            self.count.release.notified().await;
        }
        self.count
            .responses
            .lock()
            .expect("count responses")
            .pop_front()
            .unwrap_or(Ok(0))
    }

    async fn list_service_instances(
        &self,
        _: crate::GatewayServiceInstancePage,
    ) -> Result<crate::GatewayServiceInstancePageResult, GatewayEdgeError> {
        Ok(crate::GatewayServiceInstancePageResult {
            instances: Vec::new(),
            next_after: None,
        })
    }
}

pub(super) struct BlockingTargets {
    pub(super) target: Option<crate::GatewayServiceOwnedTarget>,
    pub(super) started: Notify,
    pub(super) release: Notify,
}

#[async_trait]
impl GatewayServiceTargetStore for BlockingTargets {
    async fn list_service_targets(
        &self,
        _: crate::GatewayServiceTargetPage,
    ) -> Result<crate::GatewayServiceTargetPageResult, GatewayEdgeError> {
        Ok(crate::GatewayServiceTargetPageResult {
            targets: Vec::new(),
            next_after: None,
        })
    }

    async fn get_service_target(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(self.target.clone())
    }

    async fn get_service_instance(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        Ok(None)
    }

    async fn count_accepted_service_invocations(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        Ok(0)
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        _: GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        Ok(0)
    }

    async fn list_service_instances(
        &self,
        _: crate::GatewayServiceInstancePage,
    ) -> Result<crate::GatewayServiceInstancePageResult, GatewayEdgeError> {
        Ok(crate::GatewayServiceInstancePageResult {
            instances: Vec::new(),
            next_after: None,
        })
    }
}
