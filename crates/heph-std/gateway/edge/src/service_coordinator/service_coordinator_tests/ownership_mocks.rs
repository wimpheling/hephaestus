use super::*;

pub(super) struct MockOwnership {
    pub(super) events: Mutex<Vec<&'static str>>,
    pub(super) renewals: AtomicUsize,
    pub(super) renewal_activity: Option<Arc<Notify>>,
    pub(super) renew_fails: AtomicBool,
    pub(super) stopping_fails: AtomicBool,
    pub(super) drain_conflict: AtomicBool,
    pub(super) drain_blocked: AtomicBool,
    pub(super) drain_started: Notify,
    pub(super) drain_release: Notify,
    pub(super) promote_fails: AtomicBool,
    pub(super) promote_started: Notify,
    pub(super) promote_release: Notify,
    pub(super) promote_blocked: AtomicBool,
}

pub(super) struct MockFailureStore {
    pub(super) reports: Mutex<Vec<(GatewayServiceInstanceLease, GatewayServiceFailure)>>,
    pub(super) error: Mutex<Option<GatewayServiceFailureStoreError>>,
    pub(super) started: Notify,
    pub(super) release: Notify,
    pub(super) blocked: AtomicBool,
}

#[async_trait]
impl GatewayServiceFailureStore for MockFailureStore {
    async fn record_failure(
        &self,
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        failure: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError> {
        self.started.notify_one();
        if self.blocked.load(Ordering::Relaxed) {
            self.release.notified().await;
        }
        let configured_error = *self.error.lock().expect("failure error mutex");
        if let Some(error) = configured_error {
            return Err(error);
        }
        self.reports
            .lock()
            .expect("failure reports mutex")
            .push((lease.clone(), failure));
        Ok(())
    }
}

#[async_trait]
impl GatewayServiceOwnership for MockOwnership {
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
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.renewals.fetch_add(1, Ordering::Relaxed);
        if let Some(activity) = &self.renewal_activity {
            activity.notify_one();
        }
        if self.renew_fails.load(Ordering::Relaxed) {
            Err(GatewayServiceOwnershipError::StaleLease)
        } else {
            Ok(lease.clone())
        }
    }

    async fn claim_expired(
        &self,
        _: &GatewayServiceOwner,
        _: Duration,
        _: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        Ok(Vec::new())
    }

    async fn mark_stopping(
        &self,
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.events.lock().expect("events").push("stopping");
        if self.stopping_fails.load(Ordering::Relaxed) {
            return Err(GatewayServiceOwnershipError::Unavailable);
        }
        let mut lease = lease.clone();
        lease.state = GatewayServiceInstanceState::Stopping;
        Ok(lease)
    }

    async fn mark_starting(
        &self,
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.events.lock().expect("events").push("starting");
        let mut lease = lease.clone();
        lease.state = GatewayServiceInstanceState::Starting;
        Ok(lease)
    }

    async fn mark_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.events.lock().expect("events").push("ready");
        let mut lease = lease.clone();
        lease.state = GatewayServiceInstanceState::Ready;
        Ok(lease)
    }

    async fn mark_draining(
        &self,
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.events.lock().expect("events").push("draining");
        if self.drain_conflict.load(Ordering::Relaxed) {
            return Err(GatewayServiceOwnershipError::Conflict);
        }
        self.drain_started.notify_one();
        if self.drain_blocked.load(Ordering::Relaxed) {
            self.drain_release.notified().await;
        }
        let mut lease = lease.clone();
        lease.state = GatewayServiceInstanceState::Draining;
        Ok(lease)
    }

    async fn promote_ready(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        self.events.lock().expect("events").push("promote");
        self.promote_started.notify_one();
        if self.promote_blocked.load(Ordering::Relaxed) {
            self.promote_release.notified().await;
        }
        if self.promote_fails.load(Ordering::Relaxed) {
            return Err(GatewayServiceOwnershipError::Unavailable);
        }
        Ok(None)
    }

    async fn mark_cleaned(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        self.events.lock().expect("events").push("cleaned");
        Ok(())
    }
}
