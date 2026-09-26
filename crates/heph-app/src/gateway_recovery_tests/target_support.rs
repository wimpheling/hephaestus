use super::*;

#[derive(Clone)]
pub struct ObservingTargets {
    pub inner: Arc<PostgresGatewayServiceTargets>,
    pub watched_revision: Uuid,
    pub observed: Arc<tokio::sync::Notify>,
    pub observed_scans: Arc<AtomicUsize>,
}

impl ObservingTargets {
    pub fn new(pool: sqlx::PgPool, watched_revision: Uuid) -> Self {
        Self {
            inner: Arc::new(PostgresGatewayServiceTargets::new(pool)),
            watched_revision,
            observed: Arc::new(tokio::sync::Notify::new()),
            observed_scans: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl GatewayServiceTargetStore for ObservingTargets {
    async fn list_service_targets(
        &self,
        page: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
        let result = self.inner.list_service_targets(page).await?;
        if result
            .targets
            .iter()
            .any(|target| target.desired_service_revision_id == Some(self.watched_revision))
        {
            self.observed_scans.fetch_add(1, Ordering::AcqRel);
            self.observed.notify_one();
        }
        Ok(result)
    }

    async fn get_service_target(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceOwnedTarget>, GatewayEdgeError> {
        self.inner.get_service_target(gateway_id, revision_id).await
    }

    async fn count_accepted_service_invocations(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        self.inner
            .count_accepted_service_invocations(gateway_id, revision_id)
            .await
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        key: gateway_edge::GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        self.inner
            .count_accepted_service_invocations_for_instance(key)
            .await
    }

    async fn get_service_instance(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        self.inner.get_service_instance(identity).await
    }

    async fn list_service_instances(
        &self,
        page: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
        self.inner.list_service_instances(page).await
    }
}

#[derive(Clone)]
pub struct FairnessTargets {
    pub inner: Arc<PostgresGatewayServiceTargets>,
    pub blocked_gateway: Uuid,
    pub blocked_revision: Uuid,
    pub blocked: Arc<AtomicBool>,
    pub blocked_entered: Arc<tokio::sync::Notify>,
    pub blocked_dropped: Arc<tokio::sync::Notify>,
    pub other_gets: Arc<AtomicUsize>,
    pub other_observed: Arc<tokio::sync::Notify>,
}

impl FairnessTargets {
    pub fn new(pool: sqlx::PgPool, blocked_gateway: Uuid, blocked_revision: Uuid) -> Self {
        Self {
            inner: Arc::new(PostgresGatewayServiceTargets::new(pool)),
            blocked_gateway,
            blocked_revision,
            blocked: Arc::new(AtomicBool::new(false)),
            blocked_entered: Arc::new(tokio::sync::Notify::new()),
            blocked_dropped: Arc::new(tokio::sync::Notify::new()),
            other_gets: Arc::new(AtomicUsize::new(0)),
            other_observed: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[async_trait]
impl GatewayServiceTargetStore for FairnessTargets {
    async fn list_service_targets(
        &self,
        page: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
        self.inner.list_service_targets(page).await
    }

    async fn get_service_target(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceOwnedTarget>, GatewayEdgeError> {
        if self.blocked.load(Ordering::Acquire)
            && gateway_id == self.blocked_gateway
            && revision_id == self.blocked_revision
        {
            self.blocked_entered.notify_one();
            let _drop_signal = DropSignal(Arc::clone(&self.blocked_dropped));
            std::future::pending::<()>().await;
        }
        if gateway_id != self.blocked_gateway || revision_id != self.blocked_revision {
            self.other_gets.fetch_add(1, Ordering::AcqRel);
            self.other_observed.notify_one();
        }
        self.inner.get_service_target(gateway_id, revision_id).await
    }

    async fn count_accepted_service_invocations(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        self.inner
            .count_accepted_service_invocations(gateway_id, revision_id)
            .await
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        key: gateway_edge::GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        self.inner
            .count_accepted_service_invocations_for_instance(key)
            .await
    }

    async fn get_service_instance(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        self.inner.get_service_instance(identity).await
    }

    async fn list_service_instances(
        &self,
        page: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
        self.inner.list_service_instances(page).await
    }
}

pub struct DropSignal(Arc<tokio::sync::Notify>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
