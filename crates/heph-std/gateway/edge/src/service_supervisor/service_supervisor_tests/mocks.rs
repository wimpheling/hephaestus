use super::*;
use async_trait::async_trait;
pub(super) struct Noop {
    pub(super) claim_result:
        Mutex<Option<Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>>>,
    pub(super) claim_started: Notify,
    pub(super) claim_release: Notify,
    pub(super) block_claim: AtomicBool,
    pub(super) claim_calls: AtomicUsize,
}

pub(super) struct FixedClaimResolution {
    pub(super) result:
        Mutex<Option<Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>>>,
    pub(super) calls: AtomicUsize,
}

pub(super) struct BlockingClaimResolution {
    pub(super) started: Arc<Notify>,
    pub(super) release: Arc<Notify>,
}

#[async_trait]
impl GatewayServiceClaimResolutionStore for FixedClaimResolution {
    async fn resolve_revision_claim(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.result
            .lock()
            .expect("claim resolution result")
            .clone()
            .unwrap_or(Ok(None))
    }
}

#[async_trait]
impl GatewayServiceClaimResolutionStore for BlockingClaimResolution {
    async fn resolve_revision_claim(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        self.started.notify_one();
        self.release.notified().await;
        Err(GatewayServiceOwnershipError::Unavailable)
    }
}

impl Default for Noop {
    fn default() -> Self {
        Self {
            claim_result: Mutex::new(None),
            claim_started: Notify::new(),
            claim_release: Notify::new(),
            block_claim: AtomicBool::new(false),
            claim_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl GatewayServiceOwnership for Noop {
    async fn claim_new(
        &self,
        _: Uuid,
        _: Uuid,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.claim_calls.fetch_add(1, Ordering::Relaxed);
        self.claim_started.notify_one();
        if self.block_claim.load(Ordering::Relaxed) {
            self.claim_release.notified().await;
        }
        self.claim_result
            .lock()
            .expect("claim result")
            .clone()
            .unwrap_or_else(|| unimplemented!())
    }

    async fn renew(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn claim_expired(
        &self,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
        _: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn mark_stopping(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn mark_starting(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn mark_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn mark_draining(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn promote_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        unimplemented!()
    }

    async fn mark_cleaned(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        unimplemented!()
    }
}

#[async_trait]
impl GatewayServiceFailureStore for Noop {
    async fn record_failure(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError> {
        unimplemented!()
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for Noop {
    async fn resolve_service_launch(
        &self,
        _: crate::GatewayServiceLaunchRequest,
    ) -> Result<crate::GatewayServiceLaunch, GatewayEdgeError> {
        unimplemented!()
    }

    async fn cleanup_service_launch(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        unimplemented!()
    }
}

#[async_trait]
impl GatewayServiceTargetStore for Noop {
    async fn list_service_targets(
        &self,
        _: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
        unimplemented!()
    }

    async fn get_service_target(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
        unimplemented!()
    }

    async fn count_accepted_service_invocations(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        unimplemented!()
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        _: crate::GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        unimplemented!()
    }

    async fn get_service_instance(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        unimplemented!()
    }

    async fn list_service_instances(
        &self,
        _: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
        unimplemented!()
    }
}

#[async_trait]
impl VmProvider for Noop {
    fn name(&self) -> &'static str {
        "supervisor-test-noop"
    }

    async fn provision(
        &self,
        _: vm_trait::VmSpec,
    ) -> Result<Arc<dyn vm_trait::VmInstance>, vm_trait::VmError> {
        unimplemented!()
    }

    async fn cleanup_orphan(&self, _: &vm_trait::VmId) -> Result<(), vm_trait::VmError> {
        unimplemented!()
    }
}
