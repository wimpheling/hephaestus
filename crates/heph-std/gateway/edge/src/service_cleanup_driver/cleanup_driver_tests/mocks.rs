use super::*;
use async_trait::async_trait;
pub(super) struct TestProvider {
    pub(super) gate: Option<Arc<Notify>>,
    pub(super) started: Option<Arc<Notify>>,
    pub(super) calls: AtomicUsize,
}

#[async_trait]
impl VmProvider for TestProvider {
    fn name(&self) -> &'static str {
        "cleanup-driver-test"
    }

    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Err(VmError::Destroyed)
    }

    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(started) = &self.started {
            started.notify_one();
        }
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        Ok(())
    }
}

pub(super) struct TestResolver;

#[async_trait]
impl GatewayServiceLaunchResolver for TestResolver {
    async fn resolve_service_launch(
        &self,
        _: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }

    async fn cleanup_service_launch(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        Ok(())
    }
}

pub(super) struct TestOwnership {
    pub(super) renewals: AtomicUsize,
    pub(super) mark_cleaned: AtomicUsize,
    pub(super) renewal_lease: Mutex<GatewayServiceInstanceLease>,
    pub(super) mark_result: Mutex<Result<(), GatewayServiceOwnershipError>>,
    pub(super) events: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl GatewayServiceOwnership for TestOwnership {
    async fn claim_new(
        &self,
        _: Uuid,
        _: Uuid,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn renew(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.renewals.fetch_add(1, Ordering::Relaxed);
        Ok(self.renewal_lease.lock().expect("renewal lease").clone())
    }
    async fn claim_expired(
        &self,
        _: &GatewayServiceOwner,
        _: Duration,
        _: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn mark_stopping(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn mark_starting(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn mark_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn mark_draining(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn promote_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
    async fn mark_cleaned(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        self.mark_cleaned.fetch_add(1, Ordering::Relaxed);
        self.events.lock().expect("events").push("mark_cleaned");
        *self.mark_result.lock().expect("mark result")
    }
}

pub(super) struct TestFailures {
    pub(super) calls: AtomicUsize,
    pub(super) result: Mutex<Result<(), GatewayServiceFailureStoreError>>,
    pub(super) events: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl GatewayServiceFailureStore for TestFailures {
    async fn record_failure(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.events.lock().expect("events").push("record_failure");
        *self.result.lock().expect("failure result")
    }
}

pub(super) struct TestTargets {
    pub(super) instance: Mutex<Option<GatewayServiceInstanceLease>>,
    pub(super) responses: Mutex<Vec<Option<GatewayServiceInstanceLease>>>,
}

#[async_trait]
impl GatewayServiceTargetStore for TestTargets {
    async fn list_service_targets(
        &self,
        _: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }
    async fn get_service_target(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<GatewayServiceOwnedTarget>, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }
    async fn count_accepted_service_invocations(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }
    async fn count_accepted_service_invocations_for_instance(
        &self,
        _: GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }
    async fn get_service_instance(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        let response = self.responses.lock().expect("target responses").pop();
        if let Some(response) = response {
            return Ok(response);
        }
        Ok(self.instance.lock().expect("target instance").clone())
    }
    async fn list_service_instances(
        &self,
        _: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }
}
