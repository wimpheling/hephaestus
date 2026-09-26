use super::*;

pub struct NoopLaunchResolver;

pub struct ApplicationLogLaunchResolver;

pub struct RecordingLaunchResolver {
    pub cleanup_calls: Arc<AtomicUsize>,
}

pub struct FailLaunchResolver {
    pub revision_id: Uuid,
}

#[derive(Clone)]
pub struct ObservingOwnership {
    pub inner: Arc<PostgresGatewayServiceOwnership>,
    pub claim_ack_lost_revision: Arc<Mutex<Option<Uuid>>>,
    pub claim_ack_lost_consumed: Arc<tokio::sync::Notify>,
    pub claim_absent: Arc<AtomicBool>,
    pub claim_resolution_returned: Arc<tokio::sync::Notify>,
    pub claim_attempts: Arc<Mutex<BTreeMap<Uuid, usize>>>,
    pub draining_entered: Arc<tokio::sync::Notify>,
    pub draining_returned: Arc<tokio::sync::Notify>,
    pub draining_error: Arc<Mutex<Option<GatewayServiceOwnershipError>>>,
    pub renewal_gate: Option<Arc<DestroyGate>>,
}

impl ObservingOwnership {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            inner: Arc::new(PostgresGatewayServiceOwnership::new(pool)),
            claim_ack_lost_revision: Arc::new(Mutex::new(None)),
            claim_ack_lost_consumed: Arc::new(tokio::sync::Notify::new()),
            claim_absent: Arc::new(AtomicBool::new(false)),
            claim_resolution_returned: Arc::new(tokio::sync::Notify::new()),
            claim_attempts: Arc::new(Mutex::new(BTreeMap::new())),
            draining_entered: Arc::new(tokio::sync::Notify::new()),
            draining_returned: Arc::new(tokio::sync::Notify::new()),
            draining_error: Arc::new(Mutex::new(None)),
            renewal_gate: None,
        }
    }

    pub fn with_renewal_gate(mut self, gate: Arc<DestroyGate>) -> Self {
        self.renewal_gate = Some(gate);
        self
    }

    pub fn lose_claim_ack_for_revision(&self, revision_id: Uuid) {
        *self
            .claim_ack_lost_revision
            .lock()
            .expect("claim acknowledgement fault lock") = Some(revision_id);
    }

    pub fn claim_ack_lost_consumed(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.claim_ack_lost_consumed)
    }

    pub fn confirm_next_claim_absent(&self) {
        self.claim_absent.store(true, Ordering::Release);
    }

    pub fn claim_resolution_returned(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.claim_resolution_returned)
    }

    pub fn claim_attempts(&self, revision_id: Uuid) -> usize {
        self.claim_attempts
            .lock()
            .expect("claim attempt lock")
            .get(&revision_id)
            .copied()
            .unwrap_or(0)
    }
}

#[async_trait]
impl GatewayServiceOwnership for ObservingOwnership {
    async fn claim_new(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
        owner: &GatewayServiceOwner,
        lease_duration: StdDuration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.claim_attempts
            .lock()
            .expect("claim attempt lock")
            .entry(revision_id)
            .and_modify(|attempts| *attempts += 1)
            .or_insert(1);
        if self.claim_absent.swap(false, Ordering::AcqRel) {
            return Err(GatewayServiceOwnershipError::Unavailable);
        }
        let lease = self
            .inner
            .claim_new(gateway_id, revision_id, owner, lease_duration)
            .await?;
        let lose_ack_for_revision = {
            let mut configured = self
                .claim_ack_lost_revision
                .lock()
                .expect("claim acknowledgement fault lock");
            if configured.as_ref() == Some(&revision_id) {
                configured.take();
                true
            } else {
                false
            }
        };
        if lose_ack_for_revision {
            self.claim_ack_lost_consumed.notify_one();
            return Err(GatewayServiceOwnershipError::Unavailable);
        }
        Ok(lease)
    }

    async fn renew(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        lease_duration: StdDuration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        if let Some(gate) = &self.renewal_gate {
            gate.pause_renewal_if_armed(&lease.vm_id).await;
        }
        self.inner.renew(lease, owner, lease_duration).await
    }

    async fn claim_expired(
        &self,
        owner: &GatewayServiceOwner,
        lease_duration: StdDuration,
        limit: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        self.inner.claim_expired(owner, lease_duration, limit).await
    }

    async fn mark_stopping(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.inner.mark_stopping(lease, owner).await
    }

    async fn mark_starting(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.inner.mark_starting(lease, owner).await
    }

    async fn mark_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.inner.mark_ready(lease, owner).await
    }

    async fn mark_draining(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.draining_entered.notify_one();
        let result = self.inner.mark_draining(lease, owner).await;
        *self.draining_error.lock().expect("drain observation lock") =
            result.as_ref().err().copied();
        self.draining_returned.notify_one();
        result
    }

    async fn promote_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        self.inner.promote_ready(lease, owner).await
    }

    async fn mark_cleaned(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        self.inner.mark_cleaned(lease, owner).await
    }
}

#[async_trait]
impl GatewayServiceClaimResolutionStore for ObservingOwnership {
    async fn resolve_revision_claim(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        let result = self
            .inner
            .resolve_revision_claim(gateway_id, revision_id)
            .await;
        if result.as_ref().is_ok_and(Option::is_none) {
            self.claim_resolution_returned.notify_one();
        }
        result
    }
}

#[derive(Clone)]
pub struct BlockingClaimResolution {
    pub inner: Arc<PostgresGatewayServiceOwnership>,
    pub entered: Arc<tokio::sync::Notify>,
    pub proceed: tokio::sync::watch::Receiver<bool>,
}

#[async_trait]
impl GatewayServiceClaimResolutionStore for BlockingClaimResolution {
    async fn resolve_revision_claim(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        self.entered.notify_one();
        let mut proceed = self.proceed.clone();
        while !*proceed.borrow() {
            proceed
                .changed()
                .await
                .map_err(|_| GatewayServiceOwnershipError::Unavailable)?;
        }
        self.inner
            .resolve_revision_claim(gateway_id, revision_id)
            .await
    }
}
