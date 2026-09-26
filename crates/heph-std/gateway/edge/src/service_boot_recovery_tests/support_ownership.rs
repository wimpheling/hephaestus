use crate::{
    GatewayServiceExpiredClaimRecovery, GatewayServiceInstanceLease, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError,
};
use async_trait::async_trait;
use std::time::Duration;
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use uuid::Uuid;

pub(super) struct TestOwnership {
    pub(super) claim_started: Option<Arc<Notify>>,
    pub(super) claim_release: Option<Arc<Notify>>,
    pub(super) renew_own: bool,
    pub(super) renew_calls: AtomicUsize,
    pub(super) claim_batches: Mutex<VecDeque<Vec<GatewayServiceInstanceLease>>>,
    pub(super) claim_limits: Mutex<Vec<usize>>,
    pub(super) accept_cleaned: bool,
    pub(super) cleaned: Option<Arc<Mutex<HashSet<Uuid>>>>,
    pub(super) renewed_instances: Arc<Mutex<HashSet<Uuid>>>,
    pub(super) renew_fail_after: Option<usize>,
}

impl TestOwnership {
    pub(super) fn standard() -> Self {
        Self {
            claim_started: None,
            claim_release: None,
            renew_own: false,
            renew_calls: AtomicUsize::new(0),
            claim_batches: Mutex::new(VecDeque::new()),
            claim_limits: Mutex::new(Vec::new()),
            accept_cleaned: false,
            cleaned: None,
            renewed_instances: Arc::new(Mutex::new(HashSet::new())),
            renew_fail_after: None,
        }
    }

    pub(super) fn blocking(started: Arc<Notify>, release: Arc<Notify>) -> Self {
        Self {
            claim_started: Some(started),
            claim_release: Some(release),
            ..Self::standard()
        }
    }

    pub(super) fn renew_own() -> Self {
        Self {
            renew_own: true,
            ..Self::standard()
        }
    }

    pub(super) fn batched_with_cleanup_state(
        batches: Vec<Vec<GatewayServiceInstanceLease>>,
        cleaned: Arc<Mutex<HashSet<Uuid>>>,
    ) -> Self {
        Self {
            claim_batches: Mutex::new(batches.into()),
            accept_cleaned: true,
            cleaned: Some(cleaned),
            ..Self::standard()
        }
    }
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
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        self.renew_calls.fetch_add(1, Ordering::Relaxed);
        let count = self.renew_calls.load(Ordering::Relaxed);
        if self.renew_fail_after.is_some_and(|limit| count > limit) {
            return Err(GatewayServiceOwnershipError::Unavailable);
        }
        if self.renew_own {
            self.renewed_instances
                .lock()
                .expect("renewed instances")
                .insert(lease.identity.instance_id);
        }
        self.renew_own
            .then(|| lease.clone())
            .ok_or(GatewayServiceOwnershipError::Unavailable)
    }

    async fn claim_expired(
        &self,
        _: &GatewayServiceOwner,
        _: Duration,
        limit: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        self.claim_limits.lock().expect("claim limits").push(limit);
        if let Some(started) = &self.claim_started {
            started.notify_waiters();
        }
        if let Some(release) = &self.claim_release {
            release.notified().await;
        }
        Ok(self
            .claim_batches
            .lock()
            .expect("claim batches")
            .pop_front()
            .unwrap_or_default())
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
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        if let Some(cleaned) = &self.cleaned {
            cleaned
                .lock()
                .expect("cleaned")
                .insert(lease.identity.instance_id);
        }
        self.accept_cleaned
            .then_some(())
            .ok_or(GatewayServiceOwnershipError::Unavailable)
    }
}

pub(super) struct TestExactRecovery;

#[async_trait]
impl GatewayServiceExpiredClaimRecovery for TestExactRecovery {
    async fn resolve_exact_instance(
        &self,
        _: crate::GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        Ok(None)
    }

    async fn claim_expired_instance(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Err(GatewayServiceOwnershipError::Unavailable)
    }
}
