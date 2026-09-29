use super::*;
use async_trait::async_trait;
pub(super) struct ReadyTargets {
    pub(super) target: crate::GatewayServiceOwnedTarget,
    pub(super) ownership: Arc<ReadyOwnership>,
    pub(super) accepted: Arc<AtomicUsize>,
}

#[async_trait]
impl GatewayServiceTargetStore for ReadyTargets {
    async fn list_service_targets(
        &self,
        _: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
        Ok(GatewayServiceTargetPageResult {
            targets: Vec::new(),
            next_after: None,
        })
    }

    async fn get_service_target(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<crate::GatewayServiceOwnedTarget>, GatewayEdgeError> {
        Ok(Some(self.target.clone()))
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
        _: crate::GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        Ok(self.accepted.load(Ordering::Relaxed) as u64)
    }

    async fn get_service_instance(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        Ok(Some(self.ownership.current()))
    }

    async fn list_service_instances(
        &self,
        _: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
        Ok(GatewayServiceInstancePageResult {
            instances: Vec::new(),
            next_after: None,
        })
    }
}
