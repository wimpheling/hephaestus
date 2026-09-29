use super::*;
use async_trait::async_trait;
pub(super) struct ReadyOwnership {
    pub(super) lease: Mutex<GatewayServiceInstanceLease>,
    pub(super) renew_stale: AtomicBool,
}

impl ReadyOwnership {
    pub(super) fn current(&self) -> GatewayServiceInstanceLease {
        self.lease.lock().expect("ready lease").clone()
    }

    pub(super) fn replace(&self, lease: GatewayServiceInstanceLease) {
        *self.lease.lock().expect("ready lease") = lease;
    }

    pub(super) fn transition(
        &self,
        state: crate::GatewayServiceInstanceState,
    ) -> GatewayServiceInstanceLease {
        let mut lease = self.lease.lock().expect("ready lease");
        lease.state = state;
        lease.clone()
    }
}

#[async_trait]
impl GatewayServiceOwnership for ReadyOwnership {
    async fn claim_new(
        &self,
        _: Uuid,
        _: Uuid,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Ok(self.current())
    }

    async fn renew(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        let lease = self.current();
        if self.renew_stale.load(Ordering::Relaxed)
            || lease.lease_expires_at <= OffsetDateTime::now_utc()
        {
            return Err(GatewayServiceOwnershipError::StaleLease);
        }
        Ok(lease)
    }

    async fn claim_expired(
        &self,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
        _: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        Ok(Vec::new())
    }

    async fn mark_stopping(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Ok(self.transition(crate::GatewayServiceInstanceState::Stopping))
    }

    async fn mark_starting(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Ok(self.transition(crate::GatewayServiceInstanceState::Starting))
    }

    async fn mark_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Ok(self.transition(crate::GatewayServiceInstanceState::Ready))
    }

    async fn mark_draining(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Ok(self.transition(crate::GatewayServiceInstanceState::Draining))
    }

    async fn promote_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        Ok(None)
    }

    async fn mark_cleaned(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        let _ = self.transition(crate::GatewayServiceInstanceState::Cleaned);
        Ok(())
    }
}

pub(super) struct FixedExpiredRecovery {
    pub(super) ownership: Arc<ReadyOwnership>,
    pub(super) takeover:
        Mutex<VecDeque<Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>>>,
    pub(super) resolution:
        Mutex<VecDeque<Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>>>,
    pub(super) takeover_calls: AtomicUsize,
    pub(super) resolution_calls: AtomicUsize,
}

#[async_trait]
impl GatewayServiceExpiredClaimRecovery for FixedExpiredRecovery {
    async fn resolve_exact_instance(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        self.resolution_calls.fetch_add(1, Ordering::Relaxed);
        let result = self
            .resolution
            .lock()
            .expect("resolution result")
            .pop_front()
            .unwrap_or(Ok(None));
        if let Ok(Some(lease)) = &result {
            self.ownership.replace(lease.clone());
        }
        result
    }

    async fn claim_expired_instance(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: std::time::Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.takeover_calls.fetch_add(1, Ordering::Relaxed);
        let result = self
            .takeover
            .lock()
            .expect("takeover result")
            .pop_front()
            .unwrap_or(Err(GatewayServiceOwnershipError::Unavailable));
        if let Ok(lease) = &result {
            self.ownership.replace(lease.clone());
        }
        result
    }
}
