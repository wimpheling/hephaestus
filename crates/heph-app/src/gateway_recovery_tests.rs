//! Real `PostgreSQL` proof for daemon-owned service invocation recovery.

use super::{
    gateway_reconciliation_loop, gateway_reconciliation_loop_with_boot,
    gateway_reconciliation_loop_with_context,
};
use async_trait::async_trait;
use bytes::Bytes;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayLimits,
    GatewayProvider, GatewayProviderResponse, GatewayRequest, GatewayResponse,
    GatewayServiceBootRecovery, GatewayServiceBootRecoveryContext,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanupDriverPolicy,
    GatewayServiceInstanceLease, GatewayServiceInstancePage, GatewayServiceInstancePageResult,
    GatewayServiceLaunch, GatewayServiceLaunchRequest, GatewayServiceLaunchResolver,
    GatewayServiceLogStore, GatewayServiceLogWriterConfig, GatewayServiceOwnedTarget,
    GatewayServiceOwner, GatewayServiceOwnership, GatewayServiceOwnershipError,
    GatewayServiceRegistry, GatewayServiceSupervisor, GatewayServiceSupervisorContext,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
    GatewayServiceTargetStore, ServiceLogWriterPolicy,
};
use gateway_postgres::{
    PostgresGatewayEdgeAuthority, PostgresGatewayServiceFailureStore,
    PostgresGatewayServiceLogStore, PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets,
};
use http::{HeaderMap, StatusCode};
use serial_test::serial;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{
    collections::BTreeMap,
    env, fmt,
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration as StdDuration,
};
use time::{Duration, OffsetDateTime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_trait::{
    BoxedPrivateServiceConnection, GuestCommand, NetworkMode, PrivateHttpRequest,
    PrivateHttpResponse, PrivateHttpServiceSpec, RootFilesystem, StopMode, VmError, VmEvent, VmId,
    VmInstance, VmProvider, VmResources, VmSpec,
};

#[derive(Clone, Copy)]
struct Fixture {
    owner: Uuid,
    organization: Uuid,
    project: Uuid,
    gateway: Uuid,
    revision: Uuid,
    route: Uuid,
    service_instance: Option<Uuid>,
}

type DebugServiceInstanceRow = (Uuid, String, i64, Option<String>, Option<i32>);

struct RecoveryProvider {
    reconciles: Arc<AtomicUsize>,
}

struct BlockingCaddyProvider {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    reconciles: Arc<AtomicUsize>,
}

impl RecoveryProvider {
    fn new(reconciles: Arc<AtomicUsize>) -> Self {
        Self { reconciles }
    }
}

#[derive(Clone)]
struct ServiceTransportProvider {
    inner: FakeProvider,
    provisioned: Arc<AtomicUsize>,
    destroyed: Arc<AtomicUsize>,
    destroy_gate: Option<Arc<DestroyGate>>,
    event_sender: Arc<Mutex<Option<tokio::sync::broadcast::Sender<VmEvent>>>>,
    inject_events: bool,
}

#[derive(Clone)]
struct DestroyGate {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    block_once: Arc<std::sync::atomic::AtomicBool>,
    fail_once: Arc<std::sync::atomic::AtomicBool>,
    block_retry: Arc<std::sync::atomic::AtomicBool>,
    hold_after_first_failure: Arc<std::sync::atomic::AtomicBool>,
    first_failure_release: Arc<tokio::sync::Notify>,
    renewal_pause_entered: Arc<tokio::sync::Notify>,
    renewal_pause_release: Arc<tokio::sync::Notify>,
    renewal_pause_armed: Arc<std::sync::atomic::AtomicBool>,
    retry_target_id: Arc<Mutex<Option<VmId>>>,
    attempts: Arc<AtomicUsize>,
    attempt_ids: Arc<Mutex<Vec<VmId>>>,
    orphan_attempt_ids: Arc<Mutex<Vec<VmId>>>,
}

impl DestroyGate {
    fn new() -> Self {
        Self {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
            block_once: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            fail_once: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            block_retry: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            hold_after_first_failure: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            first_failure_release: Arc::new(tokio::sync::Notify::new()),
            renewal_pause_entered: Arc::new(tokio::sync::Notify::new()),
            renewal_pause_release: Arc::new(tokio::sync::Notify::new()),
            renewal_pause_armed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            retry_target_id: Arc::new(Mutex::new(None)),
            attempts: Arc::new(AtomicUsize::new(0)),
            attempt_ids: Arc::new(Mutex::new(Vec::new())),
            orphan_attempt_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn arm_first_failure_for(&self, vm_id: VmId) {
        *self.retry_target_id.lock().expect("retry target id") = Some(vm_id);
        self.fail_once.store(true, Ordering::Release);
        self.block_once.store(false, Ordering::Release);
        self.block_retry.store(true, Ordering::Release);
    }

    fn hold_first_failure(&self) {
        self.hold_after_first_failure.store(true, Ordering::Release);
    }

    fn release_first_failure(&self) {
        self.first_failure_release.notify_one();
    }

    fn arm_renewal_pause(&self) {
        self.renewal_pause_armed.store(true, Ordering::Release);
    }

    fn renewal_pause_entered(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.renewal_pause_entered)
    }

    fn release_renewal_pause(&self) {
        self.renewal_pause_armed.store(false, Ordering::Release);
        self.renewal_pause_release.notify_one();
    }

    async fn pause_renewal_if_armed(&self, vm_id: &str) {
        let is_retry_target = self
            .retry_target_id
            .lock()
            .expect("retry target id")
            .as_ref()
            .is_some_and(|target| target.0 == vm_id);
        if is_retry_target && self.renewal_pause_armed.load(Ordering::Acquire) {
            self.renewal_pause_entered.notify_one();
            self.renewal_pause_release.notified().await;
        }
    }
}

struct ServiceTransportVm {
    inner: Arc<dyn VmInstance>,
    destroyed: Arc<AtomicUsize>,
    destroy_gate: Option<Arc<DestroyGate>>,
    events: Option<tokio::sync::broadcast::Sender<VmEvent>>,
}

#[async_trait]
impl VmProvider for ServiceTransportProvider {
    fn name(&self) -> &'static str {
        "fake-service-transport"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provisioned.fetch_add(1, Ordering::AcqRel);
        let inner = self.inner.provision(spec).await?;
        let events = if self.inject_events {
            let (events, _) = tokio::sync::broadcast::channel(64);
            *self.event_sender.lock().expect("service event sender") = Some(events.clone());
            Some(events)
        } else {
            None
        };
        Ok(Arc::new(ServiceTransportVm {
            inner,
            destroyed: Arc::clone(&self.destroyed),
            destroy_gate: self.destroy_gate.clone(),
            events,
        }))
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        if let Some(gate) = &self.destroy_gate {
            gate.orphan_attempt_ids
                .lock()
                .expect("orphan attempt ids")
                .push(id.clone());
        }
        self.inner.cleanup_orphan(id).await
    }
}

#[async_trait]
impl VmInstance for ServiceTransportVm {
    fn id(&self) -> &VmId {
        self.inner.id()
    }

    async fn start(&self) -> Result<(), VmError> {
        let result = self.inner.start().await;
        if let (Ok(()), Some(events)) = (&result, &self.events) {
            let _ = events.send(VmEvent::Started {
                ingress: Vec::new(),
            });
            let _ = events.send(VmEvent::Ready);
        }
        result
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        let result = self.inner.stop(mode).await;
        if let (Ok(()), Some(events)) = (&result, &self.events) {
            let _ = events.send(VmEvent::Log {
                stream: vm_trait::LogStream::Stderr,
                bytes: b"application-final-event".to_vec(),
            });
        }
        result
    }

    async fn wait(&self) -> Result<vm_trait::VmExit, VmError> {
        self.inner.wait().await
    }

    async fn invoke_private_http(
        &self,
        request: PrivateHttpRequest,
    ) -> Result<PrivateHttpResponse, VmError> {
        self.inner.invoke_private_http(request).await
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        let (client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 512];
            loop {
                let count = tokio::io::AsyncReadExt::read(&mut server, &mut buffer).await?;
                if count == 0 {
                    return Ok::<(), std::io::Error>(());
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    tokio::io::AsyncWriteExt::write_all(
                        &mut server,
                        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await?;
                    request.clear();
                }
            }
        });
        Ok(Box::new(client))
    }

    fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VmEvent> {
        self.events.as_ref().map_or_else(
            || self.inner.subscribe_events(),
            tokio::sync::broadcast::Sender::subscribe,
        )
    }

    async fn destroy(&self) -> Result<(), VmError> {
        if let Some(gate) = &self.destroy_gate {
            let is_retry_target = gate
                .retry_target_id
                .lock()
                .expect("retry target id")
                .as_ref()
                .is_some_and(|target| target == self.id());
            gate.attempts.fetch_add(1, Ordering::AcqRel);
            gate.attempt_ids
                .lock()
                .expect("destroy attempt ids")
                .push(self.id().clone());
            if is_retry_target && gate.fail_once.swap(false, Ordering::AcqRel) {
                gate.entered.notify_one();
                if gate.hold_after_first_failure.load(Ordering::Acquire) {
                    gate.arm_renewal_pause();
                    gate.first_failure_release.notified().await;
                }
                return Err(VmError::Unavailable {
                    resource: String::from("test-destroy"),
                    reason: String::from("injected first-attempt failure"),
                });
            }
            if is_retry_target && gate.block_retry.swap(false, Ordering::AcqRel) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        if let Some(gate) = &self.destroy_gate
            && gate.block_once.swap(false, Ordering::AcqRel)
        {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        let result = self.inner.destroy().await;
        if result.is_ok() {
            self.destroyed.fetch_add(1, Ordering::Release);
        }
        result
    }
}

struct NoopLaunchResolver;

struct ApplicationLogLaunchResolver;

struct RecordingLaunchResolver {
    cleanup_calls: Arc<AtomicUsize>,
}

struct FailLaunchResolver {
    revision_id: Uuid,
}

#[derive(Clone)]
struct ObservingOwnership {
    inner: Arc<PostgresGatewayServiceOwnership>,
    claim_ack_lost_revision: Arc<Mutex<Option<Uuid>>>,
    claim_ack_lost_consumed: Arc<tokio::sync::Notify>,
    claim_absent: Arc<AtomicBool>,
    claim_resolution_returned: Arc<tokio::sync::Notify>,
    claim_attempts: Arc<Mutex<BTreeMap<Uuid, usize>>>,
    draining_entered: Arc<tokio::sync::Notify>,
    draining_returned: Arc<tokio::sync::Notify>,
    draining_error: Arc<Mutex<Option<GatewayServiceOwnershipError>>>,
    renewal_gate: Option<Arc<DestroyGate>>,
}

impl ObservingOwnership {
    fn new(pool: sqlx::PgPool) -> Self {
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

    fn with_renewal_gate(mut self, gate: Arc<DestroyGate>) -> Self {
        self.renewal_gate = Some(gate);
        self
    }

    fn lose_claim_ack_for_revision(&self, revision_id: Uuid) {
        *self
            .claim_ack_lost_revision
            .lock()
            .expect("claim acknowledgement fault lock") = Some(revision_id);
    }

    fn claim_ack_lost_consumed(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.claim_ack_lost_consumed)
    }

    fn confirm_next_claim_absent(&self) {
        self.claim_absent.store(true, Ordering::Release);
    }

    fn claim_resolution_returned(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.claim_resolution_returned)
    }

    fn claim_attempts(&self, revision_id: Uuid) -> usize {
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
struct BlockingClaimResolution {
    inner: Arc<PostgresGatewayServiceOwnership>,
    entered: Arc<tokio::sync::Notify>,
    proceed: tokio::sync::watch::Receiver<bool>,
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

#[derive(Clone)]
struct ObservingTargets {
    inner: Arc<PostgresGatewayServiceTargets>,
    watched_revision: Uuid,
    observed: Arc<tokio::sync::Notify>,
    observed_scans: Arc<AtomicUsize>,
}

impl ObservingTargets {
    fn new(pool: sqlx::PgPool, watched_revision: Uuid) -> Self {
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
struct FairnessTargets {
    inner: Arc<PostgresGatewayServiceTargets>,
    blocked_gateway: Uuid,
    blocked_revision: Uuid,
    blocked: Arc<AtomicBool>,
    blocked_entered: Arc<tokio::sync::Notify>,
    blocked_dropped: Arc<tokio::sync::Notify>,
    other_gets: Arc<AtomicUsize>,
    other_observed: Arc<tokio::sync::Notify>,
}

impl FairnessTargets {
    fn new(pool: sqlx::PgPool, blocked_gateway: Uuid, blocked_revision: Uuid) -> Self {
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

struct DropSignal(Arc<tokio::sync::Notify>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for FailLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        if request.identity.revision_id == self.revision_id {
            return Err(GatewayEdgeError::Unavailable);
        }
        NoopLaunchResolver.resolve_service_launch(request).await
    }

    async fn cleanup_service_launch(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        NoopLaunchResolver.cleanup_service_launch(identity).await
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for NoopLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        let service = GatewayServiceConfig::new(
            18_080,
            ServiceProbePath::parse("/ready").expect("readiness path"),
            ServiceProbePath::parse("/health").expect("health path"),
        )
        .expect("service config");
        Ok(GatewayServiceLaunch {
            identity: request.identity,
            service,
            spec: VmSpec {
                id: VmId(format!("gateway-service-{}", request.identity.instance_id)),
                root: RootFilesystem::Directory {
                    host_path: PathBuf::from("/tmp"),
                },
                disks: Vec::new(),
                mounts: Vec::new(),
                resources: VmResources {
                    vcpus: 1,
                    memory_mib: 64,
                },
                network: NetworkMode::Disabled,
                private_http_service: Some(PrivateHttpServiceSpec {
                    loopback_port: 18_080,
                    max_connections: 32,
                    connect_timeout: StdDuration::from_secs(2),
                }),
                command: GuestCommand {
                    program: String::from("/service"),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    working_dir: None,
                },
                runtime_authority: None,
                runtime_git_bridge: None,
                labels: BTreeMap::new(),
            },
        })
    }

    async fn cleanup_service_launch(
        &self,
        _identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        Ok(())
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for ApplicationLogLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        let mut launch = NoopLaunchResolver.resolve_service_launch(request).await?;
        launch.service = launch
            .service
            .with_log_capture_mode(gateway_domain::ServiceLogCaptureMode::Application);
        Ok(launch)
    }

    async fn cleanup_service_launch(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        NoopLaunchResolver.cleanup_service_launch(identity).await
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for RecordingLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        NoopLaunchResolver.resolve_service_launch(request).await
    }

    async fn cleanup_service_launch(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        self.cleanup_calls.fetch_add(1, Ordering::AcqRel);
        NoopLaunchResolver.cleanup_service_launch(identity).await
    }
}

#[async_trait]
impl GatewayProvider for BlockingCaddyProvider {
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        self.reconciles.fetch_add(1, Ordering::Release);
        self.started.notify_one();
        self.release.notified().await;
        Ok(desired.revision)
    }

    async fn forward(&self, _request: GatewayRequest) -> GatewayProviderResponse {
        GatewayProviderResponse {
            response: GatewayResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                headers: HeaderMap::new(),
                body: Bytes::new(),
                mailbox_publication: None,
            },
            invocation_id: Uuid::new_v4(),
        }
    }
}

#[async_trait]
impl GatewayProvider for RecoveryProvider {
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        self.reconciles.fetch_add(1, Ordering::Release);
        Ok(desired.revision)
    }

    async fn forward(&self, _request: GatewayRequest) -> GatewayProviderResponse {
        GatewayProviderResponse {
            response: GatewayResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                headers: HeaderMap::new(),
                body: Bytes::new(),
                mailbox_publication: None,
            },
            invocation_id: Uuid::new_v4(),
        }
    }
}

/// Automatic startup loop fixture construction.
#[path = "gateway_recovery_tests/automatic_start.rs"]
mod automatic_start;
use automatic_start::{
    spawn_automatic_start_with_worker, spawn_automatic_start_with_worker_config,
};

/// Shared startup and supervisor database assertions.
#[path = "gateway_recovery_tests/recovery_support.rs"]
mod recovery_support;
use recovery_support::{
    cleanup_startup_fixture, clear_service_log_fixture, make_authority, test_supervisor_context,
    wait_for_ready, wait_for_ready_and_active,
};

/// Isolated database and worker-pool fixtures.
#[path = "gateway_recovery_tests/database.rs"]
mod database;
use database::{
    drop_isolated_startup_database, isolated_startup_database, test_pool, worker_pool,
    worker_pool_for_options_named,
};

/// Gateway and service seed fixtures.
#[path = "gateway_recovery_tests/fixtures.rs"]
mod fixtures;
use fixtures::{seed_application_log_fixture, seed_fixture};

/// Candidate release and revision fixture.
#[path = "gateway_recovery_tests/candidate.rs"]
mod candidate;
use candidate::seed_service_candidate;

/// Durable invocation, session, and secret lease fixture inserts.
#[path = "gateway_recovery_tests/persistence.rs"]
mod persistence;
use persistence::{insert_host_session, insert_invocation, insert_lease};
/// Production service log scenarios.
#[path = "gateway_recovery_tests/service_log.rs"]
mod service_log;

/// Recovery reconciliation and cancellation scenarios.
#[path = "gateway_recovery_tests/recovery_lifecycle.rs"]
mod recovery_lifecycle;

/// Supervisor polling and claim replacement scenarios.
#[path = "gateway_recovery_tests/supervisor_poll.rs"]
mod supervisor_poll;

/// Claim loss and confirmed absence scenarios.
#[path = "gateway_recovery_tests/claim_resolution.rs"]
mod claim_resolution;

/// Healthy service progress under claim contention.
#[path = "gateway_recovery_tests/healthy_progress.rs"]
mod healthy_progress;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_failed_desired_service_preserves_active_revision() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let desired_fixture = Fixture {
        revision: desired_revision,
        service_instance: None,
        ..fixture
    };
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            Some(desired_revision),
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is ready"
    );

    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let failed_attempts: i64 = sqlx::query_scalar(
                "SELECT count(*)
                   FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2 AND failure_code IS NOT NULL",
            )
            .bind(fixture.gateway)
            .bind(desired_revision)
            .fetch_one(&pool)
            .await
            .expect("read failed desired service attempt");
            if failed_attempts > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("failed desired launch is durably recorded");

    let active_revision: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("read active revision after failed desired launch");
    assert_eq!(active_revision, Some(fixture.revision));
    assert!(wait_for_ready(&pool, fixture).await);
    let desired_ready: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(desired_revision)
    .fetch_one(&pool)
    .await
    .expect("read failed desired readiness");
    assert_eq!(desired_ready, 0);
    assert_eq!(provisioned.load(Ordering::Acquire), 1);

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("failed desired loop joins")
        .expect("failed desired loop task");
    caddy_release.notify_one();
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, desired_fixture).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_promotes_desired_service_then_drains_previous_revision() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let third_revision = seed_service_candidate(&pool, fixture).await;
    let observing_targets = Arc::new(ObservingTargets::new(
        database.worker.clone(),
        third_revision,
    ));
    let destroy_gate = Arc::new(DestroyGate::new());
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            None,
            Some(observing_targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            Some(destroy_gate.clone()),
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is restored before binding the accepted invocation"
    );

    let active_instance: (Uuid, i64) = sqlx::query_as(
        "SELECT id, fencing_token
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read restored active service instance");
    let bound_fixture = Fixture {
        service_instance: Some(active_instance.0),
        ..fixture
    };
    let invocation = insert_invocation(&pool, bound_fixture, OffsetDateTime::now_utc()).await;

    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read promoted desired revision");
            if active == Some(desired_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: desired_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("desired revision becomes active and ready");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let old_state: String = sqlx::query_scalar(
                "SELECT state
                   FROM gateway_service_instances
                  WHERE id = $1",
            )
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read draining previous revision");
            if old_state == "draining" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("previous revision starts graceful drain");
    assert!(provisioned.load(Ordering::Acquire) >= 2);

    let retained_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read retained old service");
    assert_eq!(retained_state, "draining");

    let scans_before_third = observing_targets.observed_scans.load(Ordering::Acquire);
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(third_revision)
        .execute(&pool)
        .await
        .expect("declare third replacement while old service drains");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while observing_targets.observed_scans.load(Ordering::Acquire) < scans_before_third + 2 {
            observing_targets.observed.notified().await;
        }
    })
    .await
    .expect("two target refreshes observe the third desired revision");
    assert_eq!(
        provisioned.load(Ordering::Acquire),
        2,
        "a third service must wait while the old revision retains capacity"
    );
    let third_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read third replacement instances");
    assert_eq!(third_instances, 0);
    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete accepted invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove completed test invocation");
    tokio::time::timeout(StdDuration::from_secs(10), destroy_gate.entered.notified())
        .await
        .expect("old service reaches the physical destroy barrier");
    let scans_at_destroy = observing_targets.observed_scans.load(Ordering::Acquire);
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while observing_targets.observed_scans.load(Ordering::Acquire) < scans_at_destroy + 2 {
            observing_targets.observed.notified().await;
        }
    })
    .await
    .expect("two target refreshes complete while old physical cleanup is blocked");
    let old_state_while_blocked: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read old state while physical cleanup is blocked");
    assert_eq!(old_state_while_blocked, "stopping");
    let retained_instances: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read third instances while old destroy is blocked");
    assert_eq!(
        retained_instances, 0,
        "the next revision remains unclaimed while old physical cleanup is blocked"
    );
    assert_eq!(
        provisioned.load(Ordering::Acquire),
        2,
        "the next revision is not provisioned while old physical cleanup is blocked"
    );
    destroy_gate.release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(active_instance.0)
                    .fetch_one(&pool)
                    .await
                    .expect("read retired old service");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("old service cleans after accepted invocation completes");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read third promoted revision");
            if active == Some(third_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: third_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("third revision is admitted after old cleanup");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("cutover loop joins")
        .expect("cutover loop task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 2);
    cleanup_startup_fixture(&pool, fixture).await;
    drop(observing_targets);
    pool.close().await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_retries_retained_cleanup_before_admitting_next_revision() {
    daemon_loop_retries_retained_cleanup_case(false).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_recovers_expired_owned_cleanup_before_admitting_next_revision() {
    daemon_loop_retries_retained_cleanup_case(true).await;
}

// This integration case intentionally keeps the complete retained-VM lifecycle
// together so both the normal and expired-lease variants share identical setup.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn daemon_loop_retries_retained_cleanup_case(expire_before_retry: bool) {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let third_revision = seed_service_candidate(&pool, fixture).await;
    let observing_targets = Arc::new(ObservingTargets::new(
        database.worker.clone(),
        third_revision,
    ));
    let destroy_gate = Arc::new(DestroyGate::new());
    if expire_before_retry {
        destroy_gate.hold_first_failure();
    }
    let cleanup_calls = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(RecordingLaunchResolver {
        cleanup_calls: Arc::clone(&cleanup_calls),
    });
    let ownership_override = expire_before_retry.then(|| {
        Arc::new(
            ObservingOwnership::new(database.worker.clone())
                .with_renewal_gate(destroy_gate.clone()),
        ) as Arc<dyn GatewayServiceOwnership>
    });
    let (task, cancellation, caddy_started, caddy_release, _destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            ownership_override,
            Some(observing_targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            Some(destroy_gate.clone()),
            Some(resolver),
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy remains blocked while cleanup retry runs");
    assert!(wait_for_ready(&pool, fixture).await);

    let active_instance: (Uuid, i64, String, String, Uuid) = sqlx::query_as(
        "SELECT id, fencing_token, vm_id, owner_host_id, owner_uuid
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read original service instance");
    destroy_gate.arm_first_failure_for(VmId(active_instance.2.clone()));
    let invocation = insert_invocation(
        &pool,
        Fixture {
            service_instance: Some(active_instance.0),
            ..fixture
        },
        OffsetDateTime::now_utc(),
    )
    .await;
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read desired promotion");
            if active == Some(desired_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: desired_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement is ready");
    let (replacement_instance, replacement_vm_id): (Uuid, String) = sqlx::query_as(
        "SELECT id, vm_id
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(desired_revision)
    .fetch_one(&pool)
    .await
    .expect("read replacement service instance");
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(third_revision)
        .execute(&pool)
        .await
        .expect("declare next candidate");
    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete accepted invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove completed invocation");
    tokio::time::timeout(StdDuration::from_secs(10), destroy_gate.entered.notified())
        .await
        .expect("first physical cleanup attempt fails");
    assert_eq!(destroy_gate.attempts.load(Ordering::Acquire), 1);
    assert_eq!(
        destroy_gate
            .attempt_ids
            .lock()
            .expect("destroy attempt ids")
            .first()
            .map(|id| id.0.as_str()),
        Some(active_instance.2.as_str()),
    );
    if expire_before_retry {
        tokio::time::timeout(
            StdDuration::from_secs(5),
            destroy_gate.renewal_pause_entered().notified(),
        )
        .await
        .expect("A renewal pauses before database access");
        // The ownership wrapper pauses only A's renewal before it can acquire
        // the gateway lock, so B remains independently renewable.
        // Wait for the durable row to expire while the first failure is held.
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                let expired: bool = sqlx::query_scalar(
                    "SELECT clock_timestamp() >= lease_expires_at
                       FROM gateway_service_instances
                      WHERE id = $1",
                )
                .bind(active_instance.0)
                .fetch_one(&pool)
                .await
                .expect("observe retained cleanup lease expiry");
                if expired {
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(20)).await;
            }
        })
        .await
        .expect("retained cleanup lease expires while A renewal is paused");
        let (replacement_state, replacement_lease_live): (String, bool) = sqlx::query_as(
            "SELECT state, lease_expires_at > clock_timestamp()
               FROM gateway_service_instances
              WHERE id = $1",
        )
        .bind(replacement_instance)
        .fetch_one(&pool)
        .await
        .expect("read healthy B state after A expiry");
        assert_eq!(replacement_state, "ready");
        assert!(
            replacement_lease_live,
            "B lease must remain live while A expires"
        );
        let active_revision: Uuid = sqlx::query_scalar(
            "SELECT active_revision_id
               FROM gateways
              WHERE id = $1",
        )
        .bind(fixture.gateway)
        .fetch_one(&pool)
        .await
        .expect("read active B revision after A expiry");
        assert_eq!(active_revision, desired_revision);
        destroy_gate.release_first_failure();
        destroy_gate.release_renewal_pause();
    }
    tokio::time::timeout(StdDuration::from_secs(10), destroy_gate.entered.notified())
        .await
        .expect("automatic retry reaches the controlled physical cleanup barrier");
    let retry_ids = destroy_gate
        .attempt_ids
        .lock()
        .expect("destroy attempt ids")
        .clone();
    let active_vm_id = VmId(active_instance.2.clone());
    let active_attempts = retry_ids.iter().filter(|id| *id == &active_vm_id).count();
    assert!(
        active_attempts >= 2,
        "retained A cleanup must retry its exact VM; attempts={retry_ids:?}",
    );
    assert!(
        retry_ids
            .iter()
            .all(|id| id.0.as_str() != replacement_vm_id.as_str()),
        "healthy B cleanup must not be attempted while A is held; attempts={retry_ids:?}",
    );

    let retained_instance: (Uuid, i64, String, String) = sqlx::query_as(
        "SELECT id, fencing_token, vm_id, state
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(active_instance.0)
    .fetch_one(&pool)
    .await
    .expect("read retained original service claim");
    assert_eq!(retained_instance.0, active_instance.0);
    if expire_before_retry {
        assert_eq!(retained_instance.1, active_instance.1 + 1);
        let retained_owner: (String, Uuid) = sqlx::query_as(
            "SELECT owner_host_id, owner_uuid
               FROM gateway_service_instances
              WHERE id = $1",
        )
        .bind(active_instance.0)
        .fetch_one(&pool)
        .await
        .expect("read recovered cleanup owner");
        assert_eq!(retained_owner.0, active_instance.3);
        assert_eq!(retained_owner.1, active_instance.4);
    } else {
        assert_eq!(retained_instance.1, active_instance.1);
    }
    assert_eq!(retained_instance.2, active_instance.2);
    assert_eq!(retained_instance.3, "stopping");
    assert!(
        destroy_gate
            .orphan_attempt_ids
            .lock()
            .expect("orphan attempt ids")
            .is_empty(),
        "retained VM cleanup must never use the orphan path",
    );
    let scans_at_retry = observing_targets.observed_scans.load(Ordering::Acquire);
    let cleanup_heartbeat_before_retry: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(active_instance.0)
    .fetch_one(&pool)
    .await
    .expect("read retained cleanup heartbeat at retry barrier");
    let heartbeat_before_retry: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(replacement_instance)
    .fetch_one(&pool)
    .await
    .expect("read replacement heartbeat at retry barrier");

    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(replacement_instance)
            .fetch_one(&pool)
            .await
            .expect("read replacement heartbeat during retry");
            let cleanup_heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read retained cleanup heartbeat during retry");
            if heartbeat > heartbeat_before_retry
                && cleanup_heartbeat > cleanup_heartbeat_before_retry
                && observing_targets.observed_scans.load(Ordering::Acquire) >= scans_at_retry + 2
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("Caddy-blocked loop continues scans during retained cleanup");
    let third_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read candidate while cleanup retained");
    assert_eq!(third_instances, 0);
    assert_eq!(provisioned.load(Ordering::Acquire), 2);

    destroy_gate.release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(active_instance.0)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned original service");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("retry completes durable cleanup");
    assert!(cleanup_calls.load(Ordering::Acquire) > 0);
    let cleaned_instance: (Uuid, i64, String, String, OffsetDateTime) = sqlx::query_as(
        "SELECT id, fencing_token, vm_id, state, cleaned_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(active_instance.0)
    .fetch_one(&pool)
    .await
    .expect("read cleaned original service identity");
    assert_eq!(cleaned_instance.0, active_instance.0);
    if expire_before_retry {
        assert_eq!(cleaned_instance.1, active_instance.1 + 1);
    } else {
        assert_eq!(cleaned_instance.1, active_instance.1);
    }
    assert_eq!(cleaned_instance.2, active_instance.2);
    assert_eq!(cleaned_instance.3, "cleaned");
    let cleaned_at = cleaned_instance.4;
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read next active revision");
            if active == Some(third_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: third_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("next candidate admitted after durable cleanup");
    let candidate_created_at: OffsetDateTime = sqlx::query_scalar(
        "SELECT min(created_at)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read candidate creation time after cleanup");
    assert!(
        candidate_created_at >= cleaned_at,
        "candidate must be created after original instance is durably cleaned",
    );
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(replacement_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned replacement service");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement service cleans after next candidate becomes active");
    let replacement_destroyed = destroy_gate
        .attempt_ids
        .lock()
        .expect("destroy attempt ids")
        .iter()
        .any(|id| id.0 == replacement_vm_id);
    assert!(
        replacement_destroyed,
        "replacement VM must reach physical cleanup after candidate promotion",
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("cleanup retry loop joins")
        .expect("cleanup retry loop task");
    caddy_release.notify_one();
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_fairly_refreshes_other_jobs_while_one_target_lookup_times_out() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let first = seed_fixture(&pool, "http.service.v1").await;
    let second = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(second.service_instance)
        .execute(&pool)
        .await
        .expect("remove second fixture instance before automatic startup");
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(second.gateway)
    .bind(second.revision)
    .execute(&pool)
    .await
    .expect("set second active service target");
    let targets = Arc::new(FairnessTargets::new(
        database.worker.clone(),
        first.gateway,
        first.revision,
    ));
    let (task, cancellation, caddy_started, caddy_release, destroyed, _provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            first,
            true,
            false,
            None,
            None,
            None,
            Some(targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("initial Caddy reconciliation starts");
    let caddy_second = caddy_started.notified();
    caddy_release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), caddy_second)
        .await
        .expect("Caddy reconciliation makes a later pass");
    assert!(wait_for_ready(&pool, first).await, "first service is ready");
    assert!(
        wait_for_ready(&pool, second).await,
        "second service is ready"
    );

    let second_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(second.gateway)
    .bind(second.revision)
    .fetch_one(&pool)
    .await
    .expect("read second ready instance");
    let invocation = insert_invocation(
        &pool,
        Fixture {
            service_instance: Some(second_instance),
            ..second
        },
        OffsetDateTime::now_utc(),
    )
    .await;
    let first_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(first.gateway)
    .bind(first.revision)
    .fetch_one(&pool)
    .await
    .expect("read first ready instance");
    let blocked_wait = targets.blocked_entered.notified();
    targets.blocked.store(true, Ordering::Release);
    tokio::time::timeout(StdDuration::from_secs(10), blocked_wait)
        .await
        .expect("first exact target lookup is blocked");
    let heartbeat_before: OffsetDateTime =
        sqlx::query_scalar("SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1")
            .bind(first_instance)
            .fetch_one(&pool)
            .await
            .expect("read first heartbeat after blocked refresh starts");
    let other_gets_before = targets.other_gets.load(Ordering::Acquire);
    let blocked_dropped = targets.blocked_dropped.notified();
    tokio::time::timeout(StdDuration::from_secs(10), blocked_dropped)
        .await
        .expect("first exact target lookup times out and is dropped");

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(second.gateway)
        .execute(&pool)
        .await
        .expect("pause second gateway for retirement");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while targets.other_gets.load(Ordering::Acquire) <= other_gets_before {
            targets.other_observed.notified().await;
        }
    })
    .await
    .expect("refresh cursor advances to the other job");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(second_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read second draining state");
            if state == "draining" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("second service starts draining after the other refresh");

    let caddy_third = caddy_started.notified();
    caddy_release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), caddy_third)
        .await
        .expect("Caddy reconciliation progresses while target lookup is blocked");
    caddy_release.notify_one();

    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(first_instance)
            .fetch_one(&pool)
            .await
            .expect("read renewed first heartbeat");
            if heartbeat > heartbeat_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("first service lease renews while exact refresh is blocked");

    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete second accepted invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove second invocation");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(second_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned second state");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("other service cleans while first refresh is blocked");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("fair refresh loop joins")
        .expect("fair refresh loop task");
    cleanup_startup_fixture(&pool, first).await;
    cleanup_startup_fixture(&pool, second).await;
    drop_isolated_startup_database(database).await;
    assert!(destroyed.load(Ordering::Acquire) >= 2);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_starts_valid_desired_service_after_revoked_active_cleans() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is ready"
    );
    let active_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(fixture.revision)
            .fetch_one(&pool)
            .await
            .expect("read active release");
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(active_release)
        .execute(&pool)
        .await
        .expect("revoke active release");

    tokio::time::timeout(StdDuration::from_secs(20), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read replacement active revision");
            if active == Some(desired_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: desired_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("valid desired service starts after revoked active cleanup");
    assert!(provisioned.load(Ordering::Acquire) >= 2);

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("revoked-active loop joins")
        .expect("revoked-active loop task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 2);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_retries_drain_after_observed_stale_target_conflict() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    let (task, cancellation, caddy_started, caddy_release, destroyed, _provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            None,
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            None,
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is ready"
    );
    let active_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read active instance for drain hold");
    let invocation = insert_invocation(
        &pool,
        Fixture {
            service_instance: Some(active_instance),
            ..fixture
        },
        OffsetDateTime::now_utc(),
    )
    .await;

    let draining_entered = observing_ownership.draining_entered.notified();
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway before stale target read");
    let mut transition = pool.begin().await.expect("begin lifecycle transition");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transition)
        .await
        .expect("read lifecycle lock holder pid");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .fetch_one(&mut *transition)
        .await
        .expect("hold gateway transition lock");
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&mut *transition)
        .await
        .expect("restore gateway while transition lock is held");

    tokio::time::timeout(StdDuration::from_secs(10), draining_entered)
        .await
        .expect("the exact ownership adapter observed mark_draining entry");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let waiting: Option<i32> = sqlx::query_scalar(
                "SELECT pid
                   FROM pg_stat_activity
                  WHERE application_name = 'gateway-recovery-test'
                    AND wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))
                  LIMIT 1",
            )
            .bind(holder_pid)
            .fetch_optional(&pool)
            .await
            .expect("inspect stale drain lock wait");
            if waiting.is_some() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("mark_draining was blocked by the held gateway row lock");
    let draining_returned = observing_ownership.draining_returned.notified();
    transition
        .commit()
        .await
        .expect("commit concurrent lifecycle transition");

    tokio::time::timeout(StdDuration::from_secs(10), draining_returned)
        .await
        .expect("the exact mark_draining call returned after the lock release");
    assert_eq!(
        *observing_ownership
            .draining_error
            .lock()
            .expect("drain observation lock"),
        Some(GatewayServiceOwnershipError::Conflict),
        "the stale target transition must return Conflict before retrying"
    );
    let state: String = sqlx::query_scalar(
        "SELECT state FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read post-conflict service state");
    assert_eq!(state, "ready", "Conflict leaves the service Ready");

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway for retry drain");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String = sqlx::query_scalar(
                "SELECT state FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2
                  ORDER BY fencing_token DESC LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_one(&pool)
            .await
            .expect("read retried drain state");
            if state == "draining" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("later exact target refresh retries the drain");

    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete held invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove held invocation");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(active_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned retried drain state");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("retried drain cleans after invocation completion");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("stale-drain loop joins")
        .expect("stale-drain loop task");
    caddy_release.notify_one();
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn boot_gate_holds_startup_for_unexpired_owned_inventory() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let inventory = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_fixture(&pool, "http.service.v1").await;
    let host_id = format!("recovery-test-{}", candidate.gateway.simple());
    let inventory_instance = inventory
        .service_instance
        .expect("inventory fixture instance");
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(inventory_instance)
        .execute(&pool)
        .await
        .expect("replace inventory claim");
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, $6, $4, 1, $5, 'ready',
                 now() + interval '10 minutes', now())",
    )
    .bind(inventory_instance)
    .bind(inventory.gateway)
    .bind(inventory.revision)
    .bind(inventory.owner)
    .bind(format!("gateway-service-{inventory_instance}"))
    .bind(&host_id)
    .execute(&pool)
    .await
    .expect("assign live inventory to this daemon host");
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            candidate,
            true,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while boot recovery waits");
    tokio::time::sleep(StdDuration::from_secs(2)).await;
    assert_eq!(destroyed.load(Ordering::Acquire), 0);
    assert_eq!(provisioned.load(Ordering::Acquire), 0);
    let provisioned: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(candidate.gateway)
    .bind(candidate.revision)
    .fetch_one(&pool)
    .await
    .expect("read candidate claim count");
    assert_eq!(
        provisioned, 0,
        "boot inventory blocks candidate provisioning"
    );
    let inventory_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
            AND owner_host_id = $3",
    )
    .bind(inventory.gateway)
    .bind(inventory.revision)
    .bind(&host_id)
    .fetch_one(&pool)
    .await
    .expect("read retained inventory");
    assert_eq!(inventory_count, 1, "live host inventory remains present");
    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("boot-gated loop joins")
        .expect("boot-gated loop task");
    caddy_release.notify_one();
    cleanup_startup_fixture(&pool, candidate).await;
    cleanup_startup_fixture(&pool, inventory).await;
    drop_isolated_startup_database(database).await;
}
