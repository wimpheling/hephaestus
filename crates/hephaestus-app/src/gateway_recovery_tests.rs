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

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn production_service_log_writer_persists_vm_events_and_respects_disabled_mode() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let worker = database.worker.clone();
    let application = seed_application_log_fixture(&pool, "http.service.v1").await;
    let store: Arc<dyn GatewayServiceLogStore> =
        Arc::new(PostgresGatewayServiceLogStore::new(worker.clone()));
    let writer = GatewayServiceLogWriterConfig::new(store, ServiceLogWriterPolicy::default());
    let (
        application_task,
        application_cancellation,
        application_caddy_started,
        application_caddy_release,
        _application_destroyed,
        _application_provisioned,
        application_events,
    ) = spawn_automatic_start_with_worker_config(
        &pool,
        &worker,
        application,
        false,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(Arc::new(ApplicationLogLaunchResolver)),
        None,
        Some(writer.clone()),
    )
    .await;
    tokio::time::timeout(
        StdDuration::from_secs(10),
        application_caddy_started.notified(),
    )
    .await
    .expect("application reconciliation starts");
    application_caddy_release.notify_one();
    assert!(wait_for_ready(&pool, application).await);
    let application_instance: (Uuid, i64) = sqlx::query_as(
        "SELECT id, fencing_token
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(application.gateway)
    .bind(application.revision)
    .fetch_one(&pool)
    .await
    .expect("read application instance lease");
    let application_sender = tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let sender = application_events
                .lock()
                .expect("application event sender")
                .clone();
            if let Some(sender) = sender {
                break sender;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("application VM event subscriber");
    application_sender
        .send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"application-ready-event".to_vec(),
        })
        .expect("application event subscriber remains active");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*)
                   FROM gateway_service_log_chunks
                  WHERE instance_id = $1 AND fencing_token = $2",
            )
            .bind(application_instance.0)
            .bind(application_instance.1)
            .fetch_one(&pool)
            .await
            .expect("read application log chunks");
            if count > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("application event persisted before shutdown");
    application_cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), application_task)
        .await
        .expect("application reconciliation shutdown")
        .expect("application reconciliation task");
    let (application_epoch_count, application_cleaned): (i64, String) = sqlx::query_as(
        "SELECT
                (SELECT count(*) FROM gateway_service_log_epochs
                  WHERE instance_id = $1 AND fencing_token = $2),
                (SELECT state FROM gateway_service_instances WHERE id = $1)",
    )
    .bind(application_instance.0)
    .bind(application_instance.1)
    .fetch_one(&pool)
    .await
    .expect("read final application log state");
    assert_eq!(application_epoch_count, 1);
    let application_rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT sequence, stream, bytes
           FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND fencing_token = $2
          ORDER BY sequence",
    )
    .bind(application_instance.0)
    .bind(application_instance.1)
    .fetch_all(&pool)
    .await
    .expect("read ordered application log chunks");
    assert_eq!(application_rows.len(), 2);
    assert!(application_rows[0].0 < application_rows[1].0);
    assert_eq!(application_rows[0].1, "stdout");
    assert_eq!(application_rows[0].2, b"application-ready-event");
    assert_eq!(application_rows[1].1, "stderr");
    assert_eq!(application_rows[1].2, b"application-final-event");
    assert_eq!(application_cleaned, "cleaned");
    application_caddy_release.notify_one();
    clear_service_log_fixture(&pool, application).await;
    cleanup_startup_fixture(&pool, application).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn production_service_log_writer_skips_disabled_vm_events() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let worker = database.worker.clone();
    let disabled = seed_fixture(&pool, "http.service.v1").await;
    let disabled_store: Arc<dyn GatewayServiceLogStore> =
        Arc::new(PostgresGatewayServiceLogStore::new(worker.clone()));
    let disabled_writer =
        GatewayServiceLogWriterConfig::new(disabled_store, ServiceLogWriterPolicy::default());
    let (
        disabled_task,
        disabled_cancellation,
        disabled_caddy_started,
        disabled_caddy_release,
        _disabled_destroyed,
        _disabled_provisioned,
        disabled_events,
    ) = spawn_automatic_start_with_worker_config(
        &pool,
        &worker,
        disabled,
        false,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(disabled_writer),
    )
    .await;
    tokio::time::timeout(
        StdDuration::from_secs(10),
        disabled_caddy_started.notified(),
    )
    .await
    .expect("disabled reconciliation starts");
    disabled_caddy_release.notify_one();
    assert!(wait_for_ready(&pool, disabled).await);
    let disabled_instance: Uuid = sqlx::query_scalar(
        "SELECT id
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(disabled.gateway)
    .bind(disabled.revision)
    .fetch_one(&pool)
    .await
    .expect("read disabled instance");
    let disabled_sender = tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let sender = disabled_events
                .lock()
                .expect("disabled event sender")
                .clone();
            if let Some(sender) = sender {
                break sender;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("disabled VM event subscriber");
    disabled_sender
        .send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"disabled-event".to_vec(),
        })
        .expect("disabled event subscriber remains active");
    disabled_cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), disabled_task)
        .await
        .expect("disabled reconciliation shutdown")
        .expect("disabled reconciliation task");
    let disabled_epochs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_log_epochs WHERE instance_id = $1",
    )
    .bind(disabled_instance)
    .fetch_one(&pool)
    .await
    .expect("read disabled log epochs");
    assert_eq!(disabled_epochs, 0);
    disabled_caddy_release.notify_one();
    clear_service_log_fixture(&pool, disabled).await;
    cleanup_startup_fixture(&pool, disabled).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_reconciliation_reaps_abandoned_service_invocation_and_joins() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let invocation = insert_invocation(&pool, fixture, now).await;
    let session =
        insert_host_session(&pool, fixture, invocation, now, now + Duration::minutes(10)).await;
    let lease = insert_lease(&pool, fixture, invocation, session).await;
    let live = seed_fixture(&pool, "http.service.v1").await;
    let live_invocation = insert_invocation(&pool, live, now).await;
    let live_session = insert_host_session(
        &pool,
        live,
        live_invocation,
        now,
        now + Duration::minutes(10),
    )
    .await;
    let live_lease = insert_lease(&pool, live, live_invocation, live_session).await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'stopping'
          WHERE id = $1",
    )
    .bind(fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark service instance stopping");

    let reconciles = Arc::new(AtomicUsize::new(0));
    let authority = make_authority(pool.clone());
    let recovery_pool = worker_pool().await;
    let recovery_authority = make_authority(recovery_pool.clone());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop(
        authority,
        recovery_authority,
        test_supervisor_context(recovery_pool),
        Arc::new(RecoveryProvider::new(Arc::clone(&reconciles))),
        cancellation.clone(),
    ));
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let outcome: String =
                sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
                    .bind(invocation)
                    .fetch_one(&pool)
                    .await
                    .expect("read invocation outcome");
            if outcome == "timed_out" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("daemon recovery reaches the abandoned invocation");
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("read runtime session status");
    assert_eq!(session_status, "revoked");
    let lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
            .bind(lease)
            .fetch_one(&pool)
            .await
            .expect("read secret lease status");
    assert_eq!(lease_status, "revoked");
    let live_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(live.service_instance)
            .fetch_one(&pool)
            .await
            .expect("read live service state");
    assert_eq!(live_state, "ready");
    let live_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(live_invocation)
            .fetch_one(&pool)
            .await
            .expect("read live invocation outcome");
    assert_eq!(live_outcome, "accepted");
    let live_session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(live_session)
            .fetch_one(&pool)
            .await
            .expect("read live session status");
    assert_eq!(live_session_status, "active");
    let live_lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
            .bind(live_lease)
            .fetch_one(&pool)
            .await
            .expect("read live lease status");
    assert_eq!(live_lease_status, "active");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .expect("reconciliation loop joins on shutdown")
        .expect("reconciliation task join");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_recovery_cancellation_joins_a_database_blocked_batch() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let invocation = insert_invocation(&pool, fixture, now).await;
    let session =
        insert_host_session(&pool, fixture, invocation, now, now + Duration::minutes(10)).await;
    let _lease = insert_lease(&pool, fixture, invocation, session).await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'stopping'
          WHERE id = $1",
    )
    .bind(fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark service instance stopping");

    let mut lock = pool.begin().await.expect("begin authority lock");
    sqlx::query(
        "SELECT id
           FROM gateway_runtime_authority_sessions
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(session)
    .execute(&mut *lock)
    .await
    .expect("lock runtime authority session");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await
        .expect("read lock holder backend pid");

    let reconciles = Arc::new(AtomicUsize::new(0));
    let authority = make_authority(pool.clone());
    let recovery_pool = worker_pool().await;
    let recovery_authority = make_authority(recovery_pool.clone());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop(
        authority,
        recovery_authority,
        test_supervisor_context(recovery_pool),
        Arc::new(RecoveryProvider::new(Arc::clone(&reconciles))),
        cancellation.clone(),
    ));
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let waiters: Vec<(i32, String)> = sqlx::query_as(
                "SELECT pid, application_name
                   FROM pg_stat_activity
                  WHERE wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(holder_pid)
            .fetch_all(&pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            if waiters
                .iter()
                .any(|(_, application_name)| application_name == "gateway-recovery-test")
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("recovery batch waits on the held authority row");
    let reconciles_before = reconciles.load(Ordering::Acquire);
    tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let waiters: Vec<(i32, String)> = sqlx::query_as(
                "SELECT pid, application_name
                   FROM pg_stat_activity
                  WHERE wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(holder_pid)
            .fetch_all(&pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            let worker_waiting = waiters
                .iter()
                .any(|(_, application_name)| application_name == "gateway-recovery-test");
            if worker_waiting && reconciles.load(Ordering::Acquire) > reconciles_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("Caddy reconciliation advances while recovery waits");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .expect("blocked recovery loop joins on cancellation")
        .expect("reconciliation task join");
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted invocation outcome");
    assert_eq!(outcome, "accepted");
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted session status");
    assert_eq!(session_status, "active");
    let lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE invocation_id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted lease status");
    assert_eq!(lease_status, "active");
    lock.rollback().await.expect("release authority lock");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
// The supervisor is deliberately moved into the parent-owned loop future;
// its shutdown path runs there rather than at this test scope's end.
#[allow(clippy::significant_drop_tightening)]
async fn daemon_loop_polls_service_supervisor_while_caddy_reconcile_is_blocked() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2,
                active_revision_id = NULL
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(&pool)
    .await
    .expect("set desired service revision for supervisor startup");
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(fixture.service_instance)
        .execute(&pool)
        .await
        .expect("remove fixture instance before supervisor claim");

    let recovery_pool = database.worker.clone();
    let host_id = format!("recovery-test-{}", fixture.gateway.simple());
    let destroyed = Arc::new(AtomicUsize::new(0));
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(recovery_pool.clone()));
    let failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
        recovery_pool.clone(),
    ));
    let resolver = Arc::new(NoopLaunchResolver);
    let targets = Arc::new(PostgresGatewayServiceTargets::new(recovery_pool.clone()));
    let provider = Arc::new(ServiceTransportProvider {
        inner: FakeProvider::new(),
        provisioned: Arc::new(AtomicUsize::new(0)),
        destroyed: Arc::clone(&destroyed),
        destroy_gate: None,
        event_sender: Arc::new(Mutex::new(None)),
        inject_events: false,
    });
    let policy = GatewayServiceSupervisorPolicy::default();
    let owner = GatewayServiceOwner::new(host_id, Uuid::new_v4()).expect("test supervisor owner");
    let supervisor_context = GatewayServiceSupervisorContext {
        owner: owner.clone(),
        policy,
        ownership: ownership.clone(),
        failure_store: failure_store.clone(),
        resolver: resolver.clone(),
        provider: provider.clone(),
        targets: targets.clone(),
        registry: GatewayServiceRegistry::new(10, 16).expect("test service registry"),
        service_authority: String::from("127.0.0.1:8080"),
    };
    let supervisor = GatewayServiceSupervisor::new(supervisor_context)
        .expect("construct test service supervisor");
    let boot = GatewayServiceBootRecovery::new(GatewayServiceBootRecoveryContext {
        owner,
        cleanup_policy: GatewayServiceCleanupDriverPolicy {
            lease: policy.lease,
            database_timeout: policy.instance.probe_timeout,
        },
        shutdown_timeout: policy.instance.shutdown_timeout,
        ownership: ownership.clone(),
        exact_recovery: ownership.clone(),
        targets: targets.clone(),
        failure_store,
        resolver,
        provider: provider.clone(),
    })
    .expect("construct test boot recovery");
    let caddy_started = Arc::new(tokio::sync::Notify::new());
    let caddy_release = Arc::new(tokio::sync::Notify::new());
    let caddy = Arc::new(BlockingCaddyProvider {
        started: Arc::clone(&caddy_started),
        release: Arc::clone(&caddy_release),
        reconciles: Arc::new(AtomicUsize::new(0)),
    });
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop_with_boot(
        make_authority(pool.clone()),
        make_authority(recovery_pool),
        supervisor,
        Some(boot),
        None,
        None,
        targets,
        caddy,
        cancellation.clone(),
    ));

    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    let ready = tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let row: Option<(String, Option<Uuid>)> = sqlx::query_as(
                "SELECT i.state, g.active_revision_id
                   FROM gateway_service_instances AS i
                   JOIN gateways AS g ON g.id = i.gateway_id
                  WHERE i.gateway_id = $1 AND i.revision_id = $2
                  ORDER BY i.fencing_token DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_optional(&pool)
            .await
            .expect("read loop-selected service state");
            if row.as_ref().is_some_and(|(state, active_revision)| {
                state == "ready" && *active_revision == Some(fixture.revision)
            }) {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    let debug_instances: Vec<DebugServiceInstanceRow> = sqlx::query_as(
        "SELECT id, state, fencing_token, failure_code, exit_code
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_all(&pool)
    .await
    .expect("read supervisor debug state");
    assert!(
        ready.is_ok(),
        "supervisor reaches Ready while Caddy is blocked; instances={debug_instances:?}",
    );
    let active_revision: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("read promoted active revision");
    assert_eq!(active_revision, Some(fixture.revision));

    let first_heartbeat: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read ready service heartbeat");
    tokio::time::sleep(StdDuration::from_secs(6)).await;
    let second_heartbeat: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read renewed service heartbeat");
    assert!(
        second_heartbeat > first_heartbeat,
        "supervisor lease monitor must renew while Caddy is blocked"
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("supervisor and Caddy tasks join on shutdown")
        .expect("reconciliation task join");
    caddy_release.notify_one();
    let final_state: String = sqlx::query_scalar(
        "SELECT state
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY fencing_token DESC
          LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read cleaned service state");
    assert_eq!(final_state, "cleaned");
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_restores_active_service_without_manual_start() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let (task, cancellation, caddy_started, caddy_release, destroyed, _provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            false,
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
        .expect("Caddy reconciliation starts and remains blocked");
    let ready_and_active = wait_for_ready_and_active(&pool, fixture).await;
    assert!(
        ready_and_active,
        "active service is restored and promoted by the target scan: gateway={} revision={}",
        fixture.gateway, fixture.revision,
    );
    let active_revision: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("read restored active revision");
    assert_eq!(active_revision, Some(fixture.revision));
    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("active restore loop joins")
        .expect("active restore loop task");
    caddy_release.notify_one();
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_resolves_lost_claim_before_replacement_admission() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let replacement_revision = seed_service_candidate(&pool, fixture).await;
    let desired = Fixture {
        revision: desired_revision,
        ..fixture
    };
    let replacement = Fixture {
        revision: replacement_revision,
        ..fixture
    };
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    observing_ownership.lose_claim_ack_for_revision(desired_revision);
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            false,
            false,
            Some(desired_revision),
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            None,
            None,
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceClaimResolutionStore>),
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while claim resolution is pending");

    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state
                   FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2
                  ORDER BY fencing_token DESC LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(desired_revision)
            .fetch_optional(&pool)
            .await
            .expect("read resolved lost-claim state");
            if state.as_deref() == Some("cleaned") {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("committed lost claim is resolved and cleaned");

    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .execute(&pool)
    .await
    .expect("select replacement after claim cleanup");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read replacement active revision");
            if active == Some(replacement_revision) && wait_for_ready(&pool, replacement).await {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement is admitted after the resolved claim releases capacity");
    assert!(
        provisioned.load(Ordering::Acquire) >= 1,
        "replacement admission provisions a VM after lost-claim recovery"
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("lost-claim reconciliation joins")
        .expect("lost-claim reconciliation task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 1);
    cleanup_startup_fixture(&pool, desired).await;
    cleanup_startup_fixture(&pool, replacement).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_confirms_absent_claim_before_next_candidate() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let absent_revision = seed_service_candidate(&pool, fixture).await;
    let replacement_revision = seed_service_candidate(&pool, fixture).await;
    let absent = Fixture {
        revision: absent_revision,
        ..fixture
    };
    let replacement = Fixture {
        revision: replacement_revision,
        ..fixture
    };
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    observing_ownership.confirm_next_claim_absent();
    let absence_returned = observing_ownership.claim_resolution_returned();
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            false,
            false,
            Some(absent_revision),
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            None,
            None,
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceClaimResolutionStore>),
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while absent claim is resolved");
    tokio::time::timeout(StdDuration::from_secs(10), absence_returned.notified())
        .await
        .expect("serialized claim resolution returns confirmed absence");
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .execute(&pool)
    .await
    .expect("select candidate after confirmed absence");
    let absent_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(absent_revision)
    .fetch_one(&pool)
    .await
    .expect("read confirmed absent claim");
    assert_eq!(absent_count, 0, "rolled-back claim leaves no durable row");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let (active, ready_count): (Option<Uuid>, i64) = sqlx::query_as(
                "SELECT g.active_revision_id,
                        count(i.id) FILTER (WHERE i.state = 'ready')
                   FROM gateways AS g
                   LEFT JOIN gateway_service_instances AS i
                     ON i.gateway_id = g.id AND i.revision_id = $2
                  WHERE g.id = $1
                  GROUP BY g.active_revision_id",
            )
            .bind(fixture.gateway)
            .bind(replacement_revision)
            .fetch_one(&pool)
            .await
            .expect("read admitted replacement readiness");
            if active == Some(replacement_revision) && ready_count > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("next candidate starts after absent claim releases capacity");
    assert_eq!(provisioned.load(Ordering::Acquire), 1);

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("absent-claim reconciliation joins")
        .expect("absent-claim reconciliation task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 1);
    cleanup_startup_fixture(&pool, absent).await;
    cleanup_startup_fixture(&pool, replacement).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_keeps_healthy_service_progressing_while_claim_resolution_is_locked() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let replacement_revision = seed_service_candidate(&pool, fixture).await;
    let desired = Fixture {
        revision: desired_revision,
        ..fixture
    };
    let replacement = Fixture {
        revision: replacement_revision,
        ..fixture
    };
    let healthy = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query(
        "DELETE FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(healthy.gateway)
    .bind(healthy.revision)
    .execute(&pool)
    .await
    .expect("remove seeded healthy inventory before restore");
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = NULL
          WHERE id = $1",
    )
    .bind(healthy.gateway)
    .bind(healthy.revision)
    .execute(&pool)
    .await
    .expect("seed unrelated healthy service target");
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    observing_ownership.lose_claim_ack_for_revision(desired_revision);
    let claim_fault_consumed = observing_ownership.claim_ack_lost_consumed();
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("test database URL for claim-resolution observer");
    let claim_options = PgConnectOptions::from_str(&database_url)
        .expect("parse claim-resolution database URL")
        .database(&database.name);
    let claim_pool =
        worker_pool_for_options_named(claim_options, "gateway-claim-resolution-test").await;
    let claim_entered = Arc::new(tokio::sync::Notify::new());
    let (claim_proceed, claim_proceed_rx) = tokio::sync::watch::channel(false);
    let blocking_claim = Arc::new(BlockingClaimResolution {
        inner: Arc::new(PostgresGatewayServiceOwnership::new(claim_pool.clone())),
        entered: Arc::clone(&claim_entered),
        proceed: claim_proceed_rx,
    });
    let observing_targets = Arc::new(ObservingTargets::new(
        database.worker.clone(),
        replacement_revision,
    ));
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            Some(observing_targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            None,
            None,
            Some(blocking_claim as Arc<dyn GatewayServiceClaimResolutionStore>),
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while claim resolution is pending");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is ready"
    );
    assert!(
        wait_for_ready(&pool, healthy).await,
        "unrelated healthy service is ready"
    );
    let provisioned_before_replacement = provisioned.load(Ordering::Acquire);
    let active_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(healthy.gateway)
    .bind(healthy.revision)
    .fetch_one(&pool)
    .await
    .expect("read healthy active instance");

    tokio::time::timeout(StdDuration::from_secs(15), claim_fault_consumed.notified())
        .await
        .expect("desired claim acknowledgement is deliberately lost");
    let lost_claim: (Uuid, i64) = sqlx::query_as(
        "SELECT id, fencing_token
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(desired_revision)
    .fetch_one(&pool)
    .await
    .expect("capture committed lost claim identity and fence");
    tokio::time::timeout(StdDuration::from_secs(15), claim_entered.notified())
        .await
        .expect("claim resolution attempt starts");
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .execute(&pool)
    .await
    .expect("select replacement while lost claim remains unresolved");
    let mut gateway_lock = pool.begin().await.expect("begin gateway lock");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gateway_lock)
        .await
        .expect("read gateway lock backend");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .fetch_one(&mut *gateway_lock)
        .await
        .expect("hold gateway claim-resolution barrier");
    claim_proceed
        .send(true)
        .expect("open persistent claim-resolution gate");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1
                      FROM pg_stat_activity AS waiting
                     WHERE waiting.application_name = 'gateway-claim-resolution-test'
                       AND $1 = ANY(pg_blocking_pids(waiting.pid))
                )",
            )
            .bind(holder_pid)
            .fetch_one(&pool)
            .await
            .expect("observe blocked claim resolver");
            if blocked {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("claim resolution is blocked by the gateway row lock");

    let scans_before_replacement = observing_targets.observed_scans.load(Ordering::Acquire);
    let claim_attempts_before_replacement =
        observing_ownership.claim_attempts(replacement_revision);
    assert_eq!(
        claim_attempts_before_replacement, 0,
        "replacement must not be claimed before the blocked-resolution observation"
    );
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while observing_targets.observed_scans.load(Ordering::Acquire)
            < scans_before_replacement.saturating_add(2)
        {
            observing_targets.observed.notified().await;
        }
    })
    .await
    .expect("two target scans complete while claim resolution remains blocked");
    let replacement_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .fetch_one(&pool)
    .await
    .expect("read blocked replacement instances");
    assert_eq!(replacement_instances, 0);
    assert_eq!(
        observing_ownership.claim_attempts(replacement_revision),
        0,
        "replacement claim_new is not attempted while the lost claim retains capacity"
    );
    assert_eq!(
        provisioned.load(Ordering::Acquire),
        provisioned_before_replacement,
        "replacement does not provision while the lost claim retains capacity"
    );

    let heartbeat_before: OffsetDateTime =
        sqlx::query_scalar("SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1")
            .bind(active_instance)
            .fetch_one(&pool)
            .await
            .expect("read healthy heartbeat after blocker observed");
    let caddy_second = caddy_started.notified();
    caddy_release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), caddy_second)
        .await
        .expect("Caddy makes progress while claim resolution is blocked");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(active_instance)
            .fetch_one(&pool)
            .await
            .expect("read renewed healthy heartbeat");
            if heartbeat > heartbeat_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("healthy lease renews while claim resolution is blocked");
    drop(gateway_lock);

    let claim_cleaned_at: OffsetDateTime =
        tokio::time::timeout(StdDuration::from_secs(15), async {
            loop {
                let row: Option<(String, Option<OffsetDateTime>, i64)> = sqlx::query_as(
                    "SELECT state, cleaned_at, fencing_token
                   FROM gateway_service_instances
                  WHERE id = $1",
                )
                .bind(lost_claim.0)
                .fetch_optional(&pool)
                .await
                .expect("read resolved claim cleanup state");
                if let Some((state, Some(cleaned_at), fencing_token)) = row
                    && state == "cleaned"
                {
                    assert_eq!(fencing_token, lost_claim.1);
                    break cleaned_at;
                }
                tokio::time::sleep(StdDuration::from_millis(50)).await;
            }
        })
        .await
        .expect("blocked claim resolves and cleans after lock release");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let (active, ready_count): (Option<Uuid>, i64) = sqlx::query_as(
                "SELECT g.active_revision_id,
                        count(i.id) FILTER (WHERE i.state = 'ready')
                   FROM gateways AS g
                   LEFT JOIN gateway_service_instances AS i
                     ON i.gateway_id = g.id AND i.revision_id = $2
                  WHERE g.id = $1
                  GROUP BY g.active_revision_id",
            )
            .bind(fixture.gateway)
            .bind(replacement_revision)
            .fetch_one(&pool)
            .await
            .expect("read admitted replacement readiness");
            if active == Some(replacement_revision) && ready_count > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement starts after the retained claim is cleaned");
    let replacement_created_at: OffsetDateTime = sqlx::query_scalar(
        "SELECT min(created_at) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
        ",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .fetch_one(&pool)
    .await
    .expect("read replacement creation time");
    assert!(
        replacement_created_at >= claim_cleaned_at,
        "replacement must be created after the lost claim is durably cleaned"
    );
    assert!(
        observing_ownership.claim_attempts(replacement_revision)
            > claim_attempts_before_replacement,
        "replacement claim_new occurs only after cleanup releases capacity"
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("blocked claim loop joins")
        .expect("blocked claim loop task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 1);
    claim_pool.close().await;
    cleanup_startup_fixture(&pool, desired).await;
    cleanup_startup_fixture(&pool, replacement).await;
    cleanup_startup_fixture(&pool, healthy).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

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

// This fixture assembles the same durable ports as production so the loop can
// be tested without a public startup seam.
// This fixture wires each production port explicitly so tests cannot hide
// lifecycle dependencies behind a public test-only seam.
#[allow(clippy::too_many_arguments)]
async fn spawn_automatic_start_with_worker(
    pool: &sqlx::PgPool,
    worker: &sqlx::PgPool,
    fixture: Fixture,
    restore_active: bool,
    retain_inventory: bool,
    desired_revision: Option<Uuid>,
    fail_revision: Option<Uuid>,
    ownership_override: Option<Arc<dyn GatewayServiceOwnership>>,
    target_override: Option<Arc<dyn GatewayServiceTargetStore>>,
    destroy_gate: Option<Arc<DestroyGate>>,
    resolver_override: Option<Arc<dyn GatewayServiceLaunchResolver>>,
    claim_resolution_override: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
) -> (
    tokio::task::JoinHandle<()>,
    CancellationToken,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned, _) =
        spawn_automatic_start_with_worker_config(
            pool,
            worker,
            fixture,
            restore_active,
            retain_inventory,
            desired_revision,
            fail_revision,
            ownership_override,
            target_override,
            destroy_gate,
            resolver_override,
            claim_resolution_override,
            None,
        )
        .await;
    (
        task,
        cancellation,
        caddy_started,
        caddy_release,
        destroyed,
        provisioned,
    )
}

// This helper deliberately moves the supervisor context into the parent loop
// future so its cleanup handles remain live until that loop settles.
#[allow(
    clippy::significant_drop_tightening,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn spawn_automatic_start_with_worker_config(
    pool: &sqlx::PgPool,
    worker: &sqlx::PgPool,
    fixture: Fixture,
    restore_active: bool,
    retain_inventory: bool,
    desired_revision: Option<Uuid>,
    fail_revision: Option<Uuid>,
    ownership_override: Option<Arc<dyn GatewayServiceOwnership>>,
    target_override: Option<Arc<dyn GatewayServiceTargetStore>>,
    destroy_gate: Option<Arc<DestroyGate>>,
    resolver_override: Option<Arc<dyn GatewayServiceLaunchResolver>>,
    claim_resolution_override: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
    log_writer: Option<GatewayServiceLogWriterConfig>,
) -> (
    tokio::task::JoinHandle<()>,
    CancellationToken,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<Mutex<Option<tokio::sync::broadcast::Sender<VmEvent>>>>,
) {
    let host_id = format!("recovery-test-{}", fixture.gateway.simple());
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2,
                desired_service_revision_id = $3
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(if restore_active {
        Some(fixture.revision)
    } else {
        None
    })
    .bind(desired_revision.or_else(|| (!restore_active).then_some(fixture.revision)))
    .execute(pool)
    .await
    .expect("set automatic startup target");
    if retain_inventory {
        let instance = fixture
            .service_instance
            .expect("inventory fixture instance");
        sqlx::query(
            "DELETE FROM gateway_service_instances
              WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .execute(pool)
        .await
        .expect("replace inventory fixture");
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, $6, $4, 1, $5, 'ready',
                     now() + interval '10 minutes', now())",
        )
        .bind(instance)
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .bind(fixture.owner)
        .bind(format!("gateway-service-{instance}"))
        .bind(&host_id)
        .execute(pool)
        .await
        .expect("insert unexpired host inventory");
    } else {
        sqlx::query(
            "DELETE FROM gateway_service_instances
              WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .execute(pool)
        .await
        .expect("remove instance before automatic startup");
    }

    let recovery_pool = worker.clone();
    let destroyed = Arc::new(AtomicUsize::new(0));
    let provisioned = Arc::new(AtomicUsize::new(0));
    let postgres_ownership = Arc::new(PostgresGatewayServiceOwnership::new(recovery_pool.clone()));
    // The observer wraps ordinary ownership calls only; exact expired-claim
    // takeover must continue using the real Postgres recovery adapter.
    let ownership: Arc<dyn GatewayServiceOwnership> =
        ownership_override.unwrap_or_else(|| postgres_ownership.clone());
    let failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
        recovery_pool.clone(),
    ));
    let resolver: Arc<dyn GatewayServiceLaunchResolver> =
        resolver_override.unwrap_or_else(|| match fail_revision {
            Some(revision_id) => Arc::new(FailLaunchResolver { revision_id }),
            None => Arc::new(NoopLaunchResolver),
        });
    let postgres_targets = Arc::new(PostgresGatewayServiceTargets::new(recovery_pool.clone()));
    let targets: Arc<dyn GatewayServiceTargetStore> =
        target_override.unwrap_or_else(|| postgres_targets.clone());
    let limited_registry = claim_resolution_override.is_some();
    let claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore> =
        claim_resolution_override.unwrap_or_else(|| postgres_ownership.clone());
    let short_cleanup_lease = destroy_gate
        .as_ref()
        .is_some_and(|gate| gate.hold_after_first_failure.load(Ordering::Acquire));
    let provider = Arc::new(ServiceTransportProvider {
        inner: FakeProvider::new(),
        provisioned: Arc::clone(&provisioned),
        destroyed: Arc::clone(&destroyed),
        destroy_gate,
        event_sender: Arc::new(Mutex::new(None)),
        inject_events: log_writer.is_some(),
    });
    let mut policy = GatewayServiceSupervisorPolicy::default();
    if short_cleanup_lease {
        // Keep the normal heartbeat path, but make expiry observable while the
        // first physical failure is held behind the test gate.
        policy.lease.lease_duration = StdDuration::from_secs(1);
        policy.lease.renewal_interval = StdDuration::from_millis(100);
    }
    if limited_registry {
        policy.serving_gateway_capacity = 2;
        policy.replacement_capacity = 1;
    }
    let owner =
        GatewayServiceOwner::new(host_id.clone(), Uuid::new_v4()).expect("automatic startup owner");
    let registry_capacity = if limited_registry { 3 } else { 10 };
    let supervisor_context = GatewayServiceSupervisorContext {
        owner: owner.clone(),
        policy,
        ownership: ownership.clone(),
        failure_store: failure_store.clone(),
        resolver: resolver.clone(),
        provider: provider.clone(),
        targets: targets.clone(),
        registry: GatewayServiceRegistry::new(registry_capacity, 16)
            .expect("automatic startup registry"),
        service_authority: String::from("127.0.0.1:8080"),
    };
    let boot = GatewayServiceBootRecovery::new(GatewayServiceBootRecoveryContext {
        owner,
        cleanup_policy: GatewayServiceCleanupDriverPolicy {
            lease: policy.lease,
            database_timeout: policy.instance.probe_timeout,
        },
        shutdown_timeout: policy.instance.shutdown_timeout,
        ownership: ownership.clone(),
        exact_recovery: postgres_ownership.clone(),
        targets: targets.clone(),
        failure_store,
        resolver,
        provider: provider.clone(),
    })
    .expect("automatic startup boot gate");
    let caddy_started = Arc::new(tokio::sync::Notify::new());
    let caddy_release = Arc::new(tokio::sync::Notify::new());
    let caddy = Arc::new(BlockingCaddyProvider {
        started: Arc::clone(&caddy_started),
        release: Arc::clone(&caddy_release),
        reconciles: Arc::new(AtomicUsize::new(0)),
    });
    let cancellation = CancellationToken::new();
    let event_sender = Arc::clone(&provider.event_sender);
    let task = tokio::spawn(gateway_reconciliation_loop_with_context(
        make_authority(pool.clone()),
        make_authority(recovery_pool),
        supervisor_context,
        boot,
        Some(claim_resolution),
        Some(
            postgres_ownership.clone() as Arc<dyn gateway_edge::GatewayServiceExpiredClaimRecovery>
        ),
        log_writer,
        targets,
        caddy,
        cancellation.clone(),
    ));
    (
        task,
        cancellation,
        caddy_started,
        caddy_release,
        destroyed,
        provisioned,
        event_sender,
    )
}

async fn wait_for_ready(pool: &sqlx::PgPool, fixture: Fixture) -> bool {
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state
                   FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2
                  ORDER BY fencing_token DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_optional(pool)
            .await
            .expect("read automatic startup state");
            if state.as_deref() == Some("ready") {
                return true;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn wait_for_ready_and_active(pool: &sqlx::PgPool, fixture: Fixture) -> bool {
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let ready_and_active: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1
                      FROM gateway_service_instances AS i
                      JOIN gateways AS g ON g.id = i.gateway_id
                     WHERE i.gateway_id = $1
                       AND i.revision_id = $2
                       AND i.state = 'ready'
                       AND g.active_revision_id = $2
                )",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_one(pool)
            .await
            .expect("read restored service state and active revision");
            if ready_and_active {
                return true;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn cleanup_startup_fixture(pool: &sqlx::PgPool, fixture: Fixture) {
    sqlx::query(
        "UPDATE gateways
            SET lifecycle = 'paused', active_revision_id = NULL,
                desired_service_revision_id = NULL
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .execute(pool)
    .await
    .expect("retire automatic startup fixture");
    sqlx::query(
        "DELETE FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("remove automatic startup inventory");
}

async fn clear_service_log_fixture(pool: &sqlx::PgPool, fixture: Fixture) {
    sqlx::query(
        "DELETE FROM gateway_service_log_chunks
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("clear service log chunks");
    sqlx::query(
        "DELETE FROM gateway_service_log_epochs
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(pool)
    .await
    .expect("clear service log epochs");
    sqlx::query(
        "DELETE FROM gateway_service_log_project_usage
          WHERE project_id = (SELECT project_id FROM gateways WHERE id = $1)",
    )
    .bind(fixture.gateway)
    .execute(pool)
    .await
    .expect("clear service log project usage");
}

const fn make_authority(pool: sqlx::PgPool) -> PostgresGatewayEdgeAuthority {
    PostgresGatewayEdgeAuthority::new(
        pool,
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: StdDuration::from_secs(10),
        },
    )
}

fn test_supervisor_context(pool: sqlx::PgPool) -> GatewayServiceSupervisorContext {
    test_supervisor_context_with_destroy_counter(pool).0
}

fn test_supervisor_context_with_destroy_counter(
    pool: sqlx::PgPool,
) -> (GatewayServiceSupervisorContext, Arc<AtomicUsize>) {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let context = GatewayServiceSupervisorContext {
        owner: GatewayServiceOwner::new("recovery-test-host", Uuid::new_v4())
            .expect("test supervisor owner"),
        policy: GatewayServiceSupervisorPolicy::default(),
        ownership: Arc::new(PostgresGatewayServiceOwnership::new(pool.clone())),
        failure_store: Arc::new(PostgresGatewayServiceFailureStore::new(pool.clone())),
        resolver: Arc::new(NoopLaunchResolver),
        provider: Arc::new(ServiceTransportProvider {
            inner: FakeProvider::new(),
            provisioned: Arc::new(AtomicUsize::new(0)),
            destroyed: Arc::clone(&destroyed),
            destroy_gate: None,
            event_sender: Arc::new(Mutex::new(None)),
            inject_events: false,
        }),
        targets: Arc::new(PostgresGatewayServiceTargets::new(pool)),
        registry: GatewayServiceRegistry::new(10, 16).expect("test service registry"),
        service_authority: String::from("127.0.0.1:8080"),
    };
    (context, destroyed)
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    let max_version = max_version.expect("migrations are present");
    assert!(max_version >= 76);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

struct IsolatedStartupDatabase {
    control: sqlx::PgPool,
    worker: sqlx::PgPool,
    admin: sqlx::PgPool,
    name: String,
}

#[derive(Clone, Copy)]
struct PoolTeardownState {
    size: u32,
    idle: usize,
    closed: bool,
}

impl fmt::Debug for PoolTeardownState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PoolTeardownState")
            .field("size", &self.size)
            .field("idle", &self.idle)
            .field("closed", &self.closed)
            .finish()
    }
}

struct IsolatedPoolTeardownState {
    control: PoolTeardownState,
    worker: PoolTeardownState,
    admin: PoolTeardownState,
}

impl fmt::Debug for IsolatedPoolTeardownState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IsolatedPoolTeardownState")
            .field("control", &self.control)
            .field("worker", &self.worker)
            .field("admin", &self.admin)
            .finish()
    }
}

#[derive(sqlx::FromRow)]
struct IsolatedSessionDiagnostic {
    pid: i32,
    application_name: String,
    state: String,
    backend_type: String,
    backend_start: Option<OffsetDateTime>,
    state_change: Option<OffsetDateTime>,
    wait_event_type: Option<String>,
    wait_event: Option<String>,
}

impl fmt::Debug for IsolatedSessionDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IsolatedSessionDiagnostic")
            .field("pid", &self.pid)
            .field("application_name", &self.application_name)
            .field("state", &self.state)
            .field("backend_type", &self.backend_type)
            .field("backend_start", &self.backend_start)
            .field("state_change", &self.state_change)
            .field("wait_event_type", &self.wait_event_type)
            .field("wait_event", &self.wait_event)
            .finish()
    }
}

fn pool_teardown_state(pool: &sqlx::PgPool) -> PoolTeardownState {
    PoolTeardownState {
        size: pool.size(),
        idle: pool.num_idle(),
        closed: pool.is_closed(),
    }
}

fn assert_safe_isolated_database_name(name: &str) {
    assert!(name.starts_with("hephaestus_startup_"));
    assert!(name.len() > "hephaestus_startup_".len());
    assert!(
        name.bytes()
            .all(|byte| { byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' })
    );
}

async fn isolated_startup_database() -> Option<IsolatedStartupDatabase> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let options = PgConnectOptions::from_str(&database_url).expect("parse test database URL");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(
            options
                .clone()
                .database("postgres")
                .application_name("gateway-recovery-admin"),
        )
        .await
        .expect("connect PostgreSQL maintenance database");
    let name = format!("hephaestus_startup_{}", Uuid::new_v4().simple());
    assert_safe_isolated_database_name(&name);
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&admin)
        .await
        .expect("create isolated startup database");
    let control = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(
            options
                .clone()
                .database(&name)
                .application_name("gateway-recovery-control"),
        )
        .await
        .expect("connect isolated startup database");
    sqlx::migrate!("../../migrations")
        .run(&control)
        .await
        .expect("migrate isolated startup database");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&control)
        .await
        .expect("read isolated migration version");
    let max_version = max_version.expect("isolated migrations");
    assert!(max_version >= 78);
    let worker = worker_pool_for_options(options.database(&name)).await;
    println!(
        "REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 isolated_database={name} max_migration={max_version} worker_role=hephaestus_worker"
    );
    Some(IsolatedStartupDatabase {
        control,
        worker,
        admin,
        name,
    })
}

async fn drop_isolated_startup_database(database: IsolatedStartupDatabase) {
    let before_close = IsolatedPoolTeardownState {
        control: pool_teardown_state(&database.control),
        worker: pool_teardown_state(&database.worker),
        admin: pool_teardown_state(&database.admin),
    };
    database.control.close().await;
    database.worker.close().await;
    let after_close = IsolatedPoolTeardownState {
        control: pool_teardown_state(&database.control),
        worker: pool_teardown_state(&database.worker),
        admin: pool_teardown_state(&database.admin),
    };
    // Pool::close completes client-side shutdown, but PostgreSQL can report a
    // terminated backend as idle briefly while it processes termination. Keep
    // this bounded timing-sensitive grace and the session diagnostic so the
    // fixture still asserts that the database becomes session-free.
    let deadline = tokio::time::Instant::now() + StdDuration::from_secs(10);
    let mut last_sessions: Vec<IsolatedSessionDiagnostic> = Vec::new();
    loop {
        let sessions: Vec<IsolatedSessionDiagnostic> = match tokio::time::timeout_at(
            deadline,
            sqlx::query_as(
                "SELECT pid, coalesce(application_name, '') AS application_name,
                        coalesce(state, '') AS state,
                        coalesce(backend_type, '') AS backend_type,
                        backend_start, state_change, wait_event_type, wait_event
                   FROM pg_stat_activity
                  WHERE datname = $1 AND pid <> pg_backend_pid()
                  ORDER BY pid
                  LIMIT 32",
            )
            .bind(&database.name)
            .fetch_all(&database.admin),
        )
        .await
        {
            Ok(Ok(sessions)) => sessions,
            Ok(Err(error)) => panic!(
                "inspect isolated startup database sessions failed before teardown deadline: {error}; pools before close: {before_close:?}; pools after close: {after_close:?}; last sessions: {last_sessions:?}"
            ),
            Err(error) => panic!(
                "isolated startup database session inspection timed out: {error}; pools before close: {before_close:?}; pools after close: {after_close:?}; last sessions: {last_sessions:?}"
            ),
        };
        if sessions.is_empty() {
            break;
        }
        last_sessions = sessions;
        assert!(
            tokio::time::Instant::now() < deadline,
            "isolated startup database still has sessions after pool shutdown: {last_sessions:?}; pools before close: {before_close:?}; pools after close: {after_close:?}"
        );
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => {
                panic!("isolated startup database still has sessions after pool shutdown: {last_sessions:?}; pools before close: {before_close:?}; pools after close: {after_close:?}");
            }
            () = tokio::time::sleep(StdDuration::from_millis(25)) => {}
        }
    }
    assert_safe_isolated_database_name(&database.name);
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\"", database.name))
        .execute(&database.admin)
        .await
        .expect("drop isolated startup database");
    database.admin.close().await;
}

async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    let options = PgConnectOptions::from_str(&database_url).expect("parse worker test URL");
    worker_pool_for_options(options).await
}

async fn worker_pool_for_options(options: PgConnectOptions) -> sqlx::PgPool {
    worker_pool_for_options_named(options, "gateway-recovery-test").await
}

async fn worker_pool_for_options_named(
    options: PgConnectOptions,
    application_name: &'static str,
) -> sqlx::PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SELECT set_config('application_name', $1, false)")
                    .bind(application_name)
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .expect("connect worker PostgreSQL pool");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("read worker current user");
    assert_eq!(current_user, "hephaestus_worker");
    pool
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
async fn seed_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    seed_fixture_with_lease(pool, contract, false).await
}

async fn seed_application_log_fixture(pool: &sqlx::PgPool, contract: &str) -> Fixture {
    seed_fixture_with_capture_mode(pool, contract, false, "application").await
}

#[allow(clippy::too_many_lines)]
async fn seed_fixture_with_lease(
    pool: &sqlx::PgPool,
    contract: &str,
    expired_instance: bool,
) -> Fixture {
    seed_fixture_with_capture_mode(pool, contract, expired_instance, "disabled").await
}

#[allow(clippy::too_many_lines)]
async fn seed_fixture_with_capture_mode(
    pool: &sqlx::PgPool,
    contract: &str,
    expired_instance: bool,
    capture_mode: &str,
) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let release = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'recovery owner')")
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("recovery-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("recovery-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("recovery-{repository}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'enabled', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("recovery-{gateway}"))
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");
    let service = contract == "http.service.v1";
    if service {
        let source_commit = format!("{:040x}", release.as_u128());
        sqlx::query(
            "INSERT INTO build_requests
                (id, repository_id, source_commit, source_ref,
                 build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
        )
        .bind(build_request)
        .bind(repository)
        .bind(&source_commit)
        .bind([4_u8; 32].as_slice())
        .bind(owner)
        .execute(pool)
        .await
        .expect("build request");
        sqlx::query(
            "INSERT INTO releases
                (id, repository_id, version, source_commit, source_ref,
                 build_request_id, build_definition_hash, configuration,
                 configuration_hash, manifest_hash, state,
                 publication_actor_id, published_at)
             VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                     $7, $8, 'published', $9, now())",
        )
        .bind(release)
        .bind(repository)
        .bind(format!("recovery-{}", release.simple()))
        .bind(&source_commit)
        .bind(build_request)
        .bind([4_u8; 32].as_slice())
        .bind([5_u8; 32].as_slice())
        .bind([6_u8; 32].as_slice())
        .bind(owner)
        .execute(pool)
        .await
        .expect("published release");
        sqlx::query(
            "INSERT INTO agent_families (id, repository_id, agent_key)
             VALUES ($1, $2, $3)",
        )
        .bind(agent_family)
        .bind(repository)
        .bind(format!("recovery-agent-{}", release.simple()))
        .execute(pool)
        .await
        .expect("agent family");
        sqlx::query(
            "INSERT INTO release_agents
                (id, release_id, family_id, agent_key, display_name,
                 runtime_contract, runtime_contract_hash, parameter_schema,
                 secret_slot_schema, requires_state)
             VALUES ($1, $2, $3, 'recovery-service', 'Recovery service',
                     '{}', $4, '[]', '[]', false)",
        )
        .bind(release_agent)
        .bind(release)
        .bind(agent_family)
        .bind([7_u8; 32].as_slice())
        .execute(pool)
        .await
        .expect("release agent");
    }
    let secret_slots: Vec<&str> = if service { vec!["hook"] } else { Vec::new() };
    let service_loopback_port = service.then_some(18080_i32);
    let service_readiness_path = service.then_some("/ready");
    let service_health_path = service.then_some("/health");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port, service_readiness_path,
             service_health_path, service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5,
                 $6, $7, $8, 'public', '{}', $9, $10, $11, $12, $13, $14, $15)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(if service { Some(release) } else { None })
    .bind(if service { Some(release_agent) } else { None })
    .bind(if service {
        Some("recovery-service")
    } else {
        None
    })
    .bind(contract)
    .bind(&secret_slots)
    .bind(service_loopback_port)
    .bind(service_readiness_path)
    .bind(service_health_path)
    .bind(if service { capture_mode } else { "disabled" })
    .bind([9_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("active revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(format!("/service-{}", contract.replace('.', "-")))
    .execute(pool)
    .await
    .expect("route");
    let service_instance = if service {
        let instance = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_service_instances
                (id, gateway_id, revision_id, owner_host_id, owner_uuid,
                 fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
             VALUES ($1, $2, $3, 'recovery-host', $4, 1,
                     $5, 'ready',
                     CASE WHEN $6 THEN now() - interval '10 minutes'
                          ELSE now() + interval '10 minutes' END,
                     CASE WHEN $6 THEN now() - interval '20 minutes'
                          ELSE now() END)",
        )
        .bind(instance)
        .bind(gateway)
        .bind(revision)
        .bind(owner)
        .bind(format!("gateway-service-{instance}"))
        .bind(expired_instance)
        .execute(pool)
        .await
        .expect("ready service instance");
        Some(instance)
    } else {
        None
    };
    Fixture {
        owner,
        organization,
        project,
        gateway,
        revision,
        route,
        service_instance,
    }
}

// This fixture intentionally assembles a second complete published release
// so revocation tests can keep the replacement independently eligible.
#[allow(clippy::too_many_lines)]
async fn seed_service_candidate(pool: &sqlx::PgPool, fixture: Fixture) -> Uuid {
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let (_source_release_id, _source_release_agent_id, source_agent_key, repository_id): (
        Uuid,
        Uuid,
        String,
        Uuid,
    ) = sqlx::query_as(
        "SELECT release_id, release_agent_id, release_agent_key, repository_id
               FROM gateway_revisions
              WHERE id = $1",
    )
    .bind(fixture.revision)
    .fetch_one(pool)
    .await
    .expect("read source service revision");
    let release_id = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let agent_family = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let source_commit = format!("{:040x}", release_id.as_u128());
    let mut normalized_hash = [0_u8; 32];
    normalized_hash[..16].copy_from_slice(release_id.as_bytes());
    normalized_hash[16..].copy_from_slice(release_id.as_bytes());
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref,
             build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_request)
    .bind(repository_id)
    .bind(&source_commit)
    .bind([14_u8; 32].as_slice())
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("candidate build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref,
             build_request_id, build_definition_hash, configuration,
             configuration_hash, manifest_hash, state,
             publication_actor_id, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                 $7, $8, 'published', $9, now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind(format!("candidate-{}", release_id.simple()))
    .bind(&source_commit)
    .bind(build_request)
    .bind([14_u8; 32].as_slice())
    .bind([15_u8; 32].as_slice())
    .bind([16_u8; 32].as_slice())
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("candidate release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, $3)",
    )
    .bind(agent_family)
    .bind(repository_id)
    .bind(format!("candidate-agent-{}", release_id.simple()))
    .execute(pool)
    .await
    .expect("candidate agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name,
             runtime_contract, runtime_contract_hash, parameter_schema,
             secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, $4, 'Candidate service', '{}', $5, '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(agent_family)
    .bind(&source_agent_key)
    .bind([17_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate release agent");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id,
             release_agent_id, release_agent_key, handler_contract, exposure,
             parameters, secret_slots, service_loopback_port,
             service_readiness_path, service_health_path, normalized_hash,
             created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'http.service.v1', 'public',
                 '{}', '{hook}', 18081, '/ready', '/health', $8, $9)",
    )
    .bind(revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(&source_agent_key)
    .bind(normalized_hash.as_slice())
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("candidate service revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(format!("/candidate-service-{}", revision.simple()))
    .execute(pool)
    .await
    .expect("candidate service route");
    revision
}

async fn insert_invocation(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    accepted_at: OffsetDateTime,
) -> Uuid {
    let invocation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id, project_id,
             request_id, outcome, accepted_at,
             service_instance_id, service_instance_fencing_token)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted', $7, $8, $9)",
    )
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(accepted_at)
    .bind(fixture.service_instance)
    .bind(fixture.service_instance.map(|_| 1_i64))
    .execute(pool)
    .await
    .expect("invocation");
    invocation
}

async fn insert_host_session(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    invocation: Uuid,
    now: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> Uuid {
    let snapshot = Uuid::new_v4();
    let session = Uuid::new_v4();
    let hash = [7_u8; 32];
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshots
            (id, invocation_id, gateway_id, gateway_revision_id,
             authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot)
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(hash.as_slice())
    .execute(pool)
    .await
    .expect("snapshot");
    sqlx::query(
        "INSERT INTO gateway_runtime_authority_sessions
            (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
             identity_hash, snapshot_hash, issuance_generation, credential_hash,
             admission_mode, status, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NULL,
                 'host_mediated', 'active', $8, $9)",
    )
    .bind(session)
    .bind(snapshot)
    .bind(invocation)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(hash.as_slice())
    .bind(hash.as_slice())
    .bind(now - Duration::minutes(10))
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("host session");
    session
}

#[allow(clippy::too_many_lines)]
async fn insert_lease(
    pool: &sqlx::PgPool,
    fixture: Fixture,
    invocation: Uuid,
    session: Uuid,
) -> Uuid {
    let secret = Uuid::new_v4();
    let version = Uuid::new_v4();
    let grant = Uuid::new_v4();
    let import = Uuid::new_v4();
    let binding = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let lease = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO secrets
            (id, owner_organization_id, project_id, name, status,
             allowed_delivery_modes, created_by)
         VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], $5)",
    )
    .bind(secret)
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(format!("recovery_{:032x}", secret.as_u128()))
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("secret");
    sqlx::query(
        "INSERT INTO secret_versions
            (id, secret_id, sequence, status, algorithm, key_reference,
             data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
             associated_data_hash, content_length, created_by)
         VALUES ($1, $2, 1, 'active', 'AES-256-GCM+AES-256-GCM-KW/v1',
                 'test/key', decode(repeat('01', 12), 'hex'), decode('01', 'hex'),
                 decode(repeat('02', 12), 'hex'), decode('03', 'hex'),
                 decode(repeat('04', 32), 'hex'), 1, $3)",
    )
    .bind(version)
    .bind(secret)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("secret version");
    sqlx::query("UPDATE secrets SET active_version_id = $2 WHERE id = $1")
        .bind(secret)
        .bind(version)
        .execute(pool)
        .await
        .expect("active version");
    sqlx::query(
        "INSERT INTO secret_grants
            (id, secret_id, owner_organization_id, target_kind, target_id,
             target_project_id, delivery_modes, phases, status, created_by)
         VALUES ($1, $2, $3, 'project', $4, $4, ARRAY['brokered'],
                 ARRAY['normal'], 'active', $5)",
    )
    .bind(grant)
    .bind(secret)
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("grant");
    sqlx::query(
        "INSERT INTO secret_imports
            (id, grant_id, secret_id, target_kind, target_id, alias, status,
             accepted_by)
         VALUES ($1, $2, $3, 'project', $4, $5, 'active', $6)",
    )
    .bind(import)
    .bind(grant)
    .bind(secret)
    .bind(fixture.project)
    .bind(format!("recovery_{:032x}", import.as_u128()))
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("import");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
            (id, gateway_id, gateway_revision_id, import_id, slot_key,
             secret_version_id, status, normalized_hash)
         VALUES ($1, $2, $3, $4, 'hook', $5, 'active', $6)",
    )
    .bind(binding)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(import)
    .bind(version)
    .bind([5_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("binding");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
            (id, binding_id, gateway_revision_id, gateway_route_id,
             header_name, normalized_hash)
         VALUES ($1, $2, $3, $4, 'x-hook-secret', $5)",
    )
    .bind(rule)
    .bind(binding)
    .bind(fixture.revision)
    .bind(fixture.route)
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("rule");
    sqlx::query(
        "INSERT INTO gateway_secret_leases
            (id, runtime_session_id, invocation_id, binding_id,
             secret_version_id, rule_id, status, expires_at)
         SELECT $1, $2, $3, $4, $5, $6, 'active', session.expires_at
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.id = $2",
    )
    .bind(lease)
    .bind(session)
    .bind(invocation)
    .bind(binding)
    .bind(version)
    .bind(rule)
    .execute(pool)
    .await
    .expect("lease");
    lease
}
