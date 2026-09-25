//! Bounded in-memory registry for ready persistent gateway service instances.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::GatewayServiceInstanceKey;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use vm_trait::{VmId, VmInstance};

use crate::{
    GatewayEdgeError, GatewayRequest, GatewayResponse, ServiceHttpPolicy, ServiceWorkerState,
    exchange_private_service_http,
};

/// Maximum number of live service instances held by one registry.
pub const MAX_SERVICE_REGISTRY_CAPACITY: usize = 128;
/// Maximum concurrent HTTP exchanges admitted to one service instance.
pub const MAX_SERVICE_REQUEST_CAPACITY: usize = 31;

/// Redacted registry admission and lifecycle errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GatewayServiceRegistryError {
    /// The key contains a nil identity or zero fencing token.
    #[error("invalid gateway service registry key")]
    InvalidKey,
    /// A registry or per-instance capacity is outside the reviewed bound.
    #[error("invalid gateway service registry capacity")]
    InvalidCapacity,
    /// The VM identifier does not match the service instance identity.
    #[error("gateway service VM identity does not match the registry key")]
    VmIdentityMismatch,
    /// Registration requires a worker that has reached readiness.
    #[error("gateway service is not ready")]
    NotReady,
    /// The exact key is already registered.
    #[error("gateway service instance is already registered")]
    Duplicate,
    /// The registry has no free instance slot or request slot.
    #[error("gateway service capacity is exhausted")]
    CapacityExhausted,
    /// The exact key is not registered.
    #[error("gateway service instance is not registered")]
    NotFound,
}

/// Bounded registry of ready service VMs. The registry retains no worker
/// control handle and is not an authorization boundary.
#[derive(Clone)]
pub struct GatewayServiceRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    entries: RwLock<HashMap<GatewayServiceInstanceKey, Arc<RegistryEntry>>>,
    instance_slots: Arc<Semaphore>,
    requests_per_instance: usize,
}

struct RegistryEntry {
    vm: Arc<dyn VmInstance>,
    state: watch::Receiver<ServiceWorkerState>,
    cancellation: CancellationToken,
    request_slots: Arc<Semaphore>,
    _instance_slot: OwnedSemaphorePermit,
}

impl GatewayServiceRegistry {
    /// Creates a registry with explicit bounded instance and request capacity.
    ///
    /// # Errors
    ///
    /// Returns an error when either capacity is outside the reviewed bound.
    pub fn new(
        instance_capacity: usize,
        requests_per_instance: usize,
    ) -> Result<Self, GatewayServiceRegistryError> {
        if !(1..=MAX_SERVICE_REGISTRY_CAPACITY).contains(&instance_capacity)
            || !(1..=MAX_SERVICE_REQUEST_CAPACITY).contains(&requests_per_instance)
        {
            return Err(GatewayServiceRegistryError::InvalidCapacity);
        }
        Ok(Self {
            inner: Arc::new(RegistryInner {
                entries: RwLock::new(HashMap::new()),
                instance_slots: Arc::new(Semaphore::new(instance_capacity)),
                requests_per_instance,
            }),
        })
    }

    /// Registers one ready VM under its exact fenced identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the key, VM identity, readiness, duplicate, or
    /// registry capacity check fails.
    pub fn register(
        &self,
        key: GatewayServiceInstanceKey,
        vm: Arc<dyn VmInstance>,
        state: watch::Receiver<ServiceWorkerState>,
    ) -> Result<(), GatewayServiceRegistryError> {
        if !key.is_valid() {
            return Err(GatewayServiceRegistryError::InvalidKey);
        }
        let expected = format!("gateway-service-{}", key.identity.instance_id);
        if vm.id() != &VmId(expected) {
            return Err(GatewayServiceRegistryError::VmIdentityMismatch);
        }
        if state.has_changed().is_err() || *state.borrow() != ServiceWorkerState::Ready {
            return Err(GatewayServiceRegistryError::NotReady);
        }
        {
            let entries = self.read_entries();
            if entries
                .keys()
                .any(|existing| existing.identity.instance_id == key.identity.instance_id)
            {
                return Err(GatewayServiceRegistryError::Duplicate);
            }
        }
        let instance_slot = self
            .inner
            .instance_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| GatewayServiceRegistryError::CapacityExhausted)?;
        let mut entries = self.write_entries();
        if entries
            .keys()
            .any(|existing| existing.identity.instance_id == key.identity.instance_id)
        {
            drop(instance_slot);
            return Err(GatewayServiceRegistryError::Duplicate);
        }
        entries.insert(
            key,
            Arc::new(RegistryEntry {
                vm,
                state,
                cancellation: CancellationToken::new(),
                request_slots: Arc::new(Semaphore::new(self.inner.requests_per_instance)),
                _instance_slot: instance_slot,
            }),
        );
        drop(entries);
        Ok(())
    }

    /// Unregisters exactly one fenced key and cancels its active exchanges.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceRegistryError::NotFound`] for a stale key.
    pub fn unregister(
        &self,
        key: GatewayServiceInstanceKey,
    ) -> Result<(), GatewayServiceRegistryError> {
        if !key.is_valid() {
            return Err(GatewayServiceRegistryError::InvalidKey);
        }
        let entry = {
            let mut entries = self.write_entries();
            entries
                .remove(&key)
                .ok_or(GatewayServiceRegistryError::NotFound)?
        };
        entry.cancellation.cancel();
        drop(entry);
        Ok(())
    }

    /// Exchanges one request through the exact fenced ready service instance.
    ///
    /// The registry performs admission only. Callers must authorize the
    /// request and fencing token through the durable control plane first.
    ///
    /// # Errors
    ///
    /// Returns a contract error for an invalid key or policy and an
    /// unavailable error for a missing, stopping, or saturated instance.
    // The entry is intentionally held through every await so unregister
    // cannot release its global instance slot before this request quiesces.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn exchange(
        &self,
        key: GatewayServiceInstanceKey,
        request: GatewayRequest,
        policy: ServiceHttpPolicy,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        if !key.is_valid() {
            return Err(GatewayEdgeError::Contract("invalid service instance key"));
        }
        policy.validate()?;
        let entry = {
            let entries = self.read_entries();
            entries
                .get(&key)
                .cloned()
                .ok_or(GatewayEdgeError::HandlerUnavailable)?
        };
        let mut state = entry.state.clone();
        let _request_slot = entry
            .request_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        if state.has_changed().is_err() || *state.borrow() != ServiceWorkerState::Ready {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let deadline = Instant::now()
            .checked_add(policy.exchange_timeout)
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let open = time::timeout_at(deadline, entry.vm.open_private_service_connection());
        tokio::pin!(open);
        let connection = loop {
            tokio::select! {
                () = entry.cancellation.cancelled() => {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                changed = state.changed() => {
                    if changed.is_err() || *state.borrow() != ServiceWorkerState::Ready {
                        return Err(GatewayEdgeError::HandlerUnavailable);
                    }
                }
                result = &mut open => {
                    break result
                        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
                        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
                }
            }
        };
        if state.has_changed().is_err() || *state.borrow() != ServiceWorkerState::Ready {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let mut exchange_policy = policy;
        exchange_policy.exchange_timeout = remaining;
        let exchange = time::timeout_at(
            deadline,
            exchange_private_service_http(connection, request, exchange_policy),
        );
        tokio::pin!(exchange);
        loop {
            tokio::select! {
                () = entry.cancellation.cancelled() => {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                changed = state.changed() => {
                    if changed.is_err() || *state.borrow() != ServiceWorkerState::Ready {
                        return Err(GatewayEdgeError::HandlerUnavailable);
                    }
                }
                result = &mut exchange => {
                    return result.map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
                }
            }
        }
    }

    fn read_entries(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashMap<GatewayServiceInstanceKey, Arc<RegistryEntry>>>
    {
        self.inner
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_entries(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, HashMap<GatewayServiceInstanceKey, Arc<RegistryEntry>>>
    {
        self.inner
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use bytes::Bytes;
    use gateway_domain::GatewayServiceIdentity;
    use http::{HeaderMap, Method, StatusCode};
    use std::{
        collections::VecDeque,
        net::IpAddr,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        sync::{Barrier, Notify, broadcast, oneshot, watch},
    };
    use uuid::Uuid;
    use vm_trait::{BoxedPrivateServiceConnection, StopMode, VmError, VmEvent, VmExit};

    struct TestVm {
        id: VmId,
        connections: Mutex<VecDeque<BoxedPrivateServiceConnection>>,
        opened: Notify,
        release_open: Option<Arc<Notify>>,
        open_delay: Duration,
        events: broadcast::Sender<VmEvent>,
        opens: AtomicUsize,
    }

    impl TestVm {
        fn new(id: VmId) -> Arc<Self> {
            let (events, _) = broadcast::channel(4);
            Arc::new(Self {
                id,
                connections: Mutex::new(VecDeque::new()),
                opened: Notify::new(),
                release_open: None,
                open_delay: Duration::ZERO,
                events,
                opens: AtomicUsize::new(0),
            })
        }

        fn blocking(id: VmId, release_open: Arc<Notify>) -> Arc<Self> {
            let mut vm = Self::new(id);
            Arc::get_mut(&mut vm)
                .expect("new VM is unique")
                .release_open = Some(release_open);
            vm
        }

        fn delayed(id: VmId, open_delay: Duration) -> Arc<Self> {
            let mut vm = Self::new(id);
            Arc::get_mut(&mut vm).expect("new VM is unique").open_delay = open_delay;
            vm
        }

        fn push_connection(&self, connection: BoxedPrivateServiceConnection) {
            self.connections
                .lock()
                .expect("connection lock")
                .push_back(connection);
        }
    }

    #[async_trait]
    impl VmInstance for TestVm {
        fn id(&self) -> &VmId {
            &self.id
        }

        async fn start(&self) -> Result<(), VmError> {
            Ok(())
        }

        async fn stop(&self, _: StopMode) -> Result<(), VmError> {
            Ok(())
        }

        async fn wait(&self) -> Result<VmExit, VmError> {
            std::future::pending().await
        }

        async fn open_private_service_connection(
            &self,
        ) -> Result<BoxedPrivateServiceConnection, VmError> {
            self.opens.fetch_add(1, Ordering::Relaxed);
            self.opened.notify_one();
            tokio::time::sleep(self.open_delay).await;
            if let Some(release) = &self.release_open {
                release.notified().await;
            }
            self.connections
                .lock()
                .expect("connection lock")
                .pop_front()
                .ok_or_else(|| VmError::Unavailable {
                    resource: String::from("test connection"),
                    reason: String::from("no connection queued"),
                })
        }

        fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
            self.events.subscribe()
        }

        async fn destroy(&self) -> Result<(), VmError> {
            Ok(())
        }
    }

    fn key(instance_id: Uuid, fencing_token: i64) -> GatewayServiceInstanceKey {
        GatewayServiceInstanceKey {
            identity: GatewayServiceIdentity {
                instance_id,
                gateway_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            },
            fencing_token,
        }
    }

    fn state() -> (
        watch::Sender<ServiceWorkerState>,
        watch::Receiver<ServiceWorkerState>,
    ) {
        watch::channel(ServiceWorkerState::Ready)
    }

    fn vm_for(key: GatewayServiceInstanceKey) -> Arc<TestVm> {
        TestVm::new(VmId(format!(
            "gateway-service-{}",
            key.identity.instance_id
        )))
    }

    fn request() -> GatewayRequest {
        GatewayRequest {
            method: Method::GET,
            path_and_query: String::from("/health"),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            trusted: crate::TrustedRequestMetadata {
                scheme: crate::GatewayScheme::Https,
                authority: String::from("service.example.test"),
                client_address: IpAddr::from([127, 0, 0, 1]),
                request_id: Uuid::new_v4(),
            },
        }
    }

    fn policy() -> ServiceHttpPolicy {
        ServiceHttpPolicy::from_gateway_limits(crate::GatewayLimits {
            max_request_body_bytes: 128,
            max_response_body_bytes: 128,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: std::time::Duration::from_secs(1),
        })
    }

    async fn read_headers(stream: &mut DuplexStream) {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 512];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await.expect("read request");
            assert_ne!(count, 0, "request closed before headers");
            bytes.extend_from_slice(&buffer[..count]);
        }
    }

    fn queued_pair(vm: &TestVm) -> DuplexStream {
        let (client, server) = tokio::io::duplex(16 * 1024);
        vm.push_connection(Box::new(client));
        server
    }

    #[test]
    fn validates_capacity_identity_readiness_and_fencing() {
        assert!(GatewayServiceRegistry::new(0, 1).is_err());
        assert!(GatewayServiceRegistry::new(1, 32).is_err());
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let instance = Uuid::new_v4();
        let first = key(instance, 1);
        let second = key(Uuid::new_v4(), 1);
        let vm = vm_for(first);
        let (_states, state_receiver) = state();
        registry
            .register(first, vm.clone(), state_receiver)
            .expect("first registration");
        assert_eq!(
            registry
                .register(
                    GatewayServiceInstanceKey {
                        fencing_token: 2,
                        ..first
                    },
                    vm.clone(),
                    state().1,
                )
                .unwrap_err(),
            GatewayServiceRegistryError::Duplicate
        );
        assert_eq!(
            registry.register(first, vm, state().1).unwrap_err(),
            GatewayServiceRegistryError::Duplicate
        );
        let (_second_sender, second_state) = state();
        assert_eq!(
            registry
                .register(second, vm_for(second), second_state)
                .unwrap_err(),
            GatewayServiceRegistryError::CapacityExhausted
        );
        assert_eq!(
            registry.unregister(key(instance, 2)).unwrap_err(),
            GatewayServiceRegistryError::NotFound
        );
        registry.unregister(first).expect("remove first");
        let (_replacement_sender, replacement_state) = state();
        registry
            .register(second, vm_for(second), replacement_state)
            .expect("capacity released");
        drop(registry);
    }

    #[test]
    fn rejects_nonready_and_mismatched_vms() {
        let registry = GatewayServiceRegistry::new(2, 1).expect("registry");
        let instance_key = key(Uuid::new_v4(), 1);
        let (not_ready_sender, not_ready) = watch::channel(ServiceWorkerState::Probing);
        assert_eq!(
            registry
                .register(instance_key, vm_for(instance_key), not_ready)
                .unwrap_err(),
            GatewayServiceRegistryError::NotReady
        );
        not_ready_sender.send_replace(ServiceWorkerState::Ready);
        let wrong = TestVm::new(VmId(String::from("wrong-vm")));
        assert_eq!(
            registry
                .register(instance_key, wrong, state().1)
                .unwrap_err(),
            GatewayServiceRegistryError::VmIdentityMismatch
        );
        let (closed_sender, closed_state) = state();
        drop(closed_sender);
        let closed_key = key(Uuid::new_v4(), 1);
        assert_eq!(
            registry
                .register(closed_key, vm_for(closed_key), closed_state)
                .unwrap_err(),
            GatewayServiceRegistryError::NotReady
        );
        drop(registry);
    }

    #[tokio::test]
    async fn exchanges_over_exact_ready_instance() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let vm = vm_for(key);
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        vm.push_connection(Box::new(client));
        let (_state_sender, state_receiver) = state();
        registry
            .register(key, vm, state_receiver)
            .expect("registration");
        let mut invalid_policy = policy();
        invalid_policy.max_response_headers = 0;
        assert!(matches!(
            registry.exchange(key, request(), invalid_policy).await,
            Err(GatewayEdgeError::Contract(_))
        ));
        let server_task = tokio::spawn(async move {
            read_headers(&mut server).await;
            server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("response");
        });
        let response = registry
            .exchange(key, request(), policy())
            .await
            .expect("exchange");
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from_static(b"ok"));
        drop(registry);
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn per_instance_capacity_and_unregister_cancel_are_bounded() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let release = Arc::new(Notify::new());
        let vm = TestVm::blocking(
            VmId(format!("gateway-service-{}", key.identity.instance_id)),
            Arc::clone(&release),
        );
        let (_state_sender, state_receiver) = state();
        registry
            .register(key, vm.clone(), state_receiver)
            .expect("registration");
        let first = tokio::spawn({
            let registry = registry.clone();
            async move { registry.exchange(key, request(), policy()).await }
        });
        vm.opened.notified().await;
        assert!(matches!(
            registry.exchange(key, request(), policy()).await,
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        registry.unregister(key).expect("unregister");
        assert!(matches!(
            first.await.expect("exchange task"),
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        release.notify_one();
        drop(registry);
    }

    #[tokio::test]
    async fn caller_cancellation_releases_permit_and_closes_peer() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let vm = vm_for(key);
        let mut first_server = queued_pair(&vm);
        let mut second_server = queued_pair(&vm);
        let (_state_sender, state_receiver) = state();
        registry
            .register(key, vm, state_receiver)
            .expect("registration");
        let (headers_seen, headers_received) = oneshot::channel();
        let (peer_closed, peer_closed_received) = oneshot::channel();
        let first_server_task = tokio::spawn(async move {
            read_headers(&mut first_server).await;
            headers_seen.send(()).expect("headers signal");
            let mut byte = [0_u8; 1];
            let result = first_server.read(&mut byte).await;
            assert!(matches!(result, Ok(0) | Err(_)));
            peer_closed.send(()).expect("close signal");
        });
        let first = tokio::spawn({
            let registry = registry.clone();
            async move { registry.exchange(key, request(), policy()).await }
        });
        headers_received.await.expect("request reached peer");
        first.abort();
        assert!(first.await.expect_err("exchange cancelled").is_cancelled());
        peer_closed_received.await.expect("peer close");
        let second_server_task = tokio::spawn(async move {
            read_headers(&mut second_server).await;
            second_server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("response");
        });
        let response = registry
            .exchange(key, request(), policy())
            .await
            .expect("permit released");
        assert_eq!(response.body, Bytes::from_static(b"ok"));
        first_server_task.await.expect("first server");
        second_server_task.await.expect("second server");
        drop(registry);
    }

    #[tokio::test]
    async fn stopping_state_cancels_active_exchange_and_closes_peer() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let vm = vm_for(key);
        let mut server = queued_pair(&vm);
        let (state_sender, state_receiver) = state();
        registry
            .register(key, vm, state_receiver)
            .expect("registration");
        let (headers_seen, headers_received) = oneshot::channel();
        let (peer_closed, peer_closed_received) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_headers(&mut server).await;
            headers_seen.send(()).expect("headers signal");
            let mut byte = [0_u8; 1];
            let result = server.read(&mut byte).await;
            assert!(matches!(result, Ok(0) | Err(_)));
            peer_closed.send(()).expect("close signal");
        });
        let exchange = tokio::spawn({
            let registry = registry.clone();
            async move { registry.exchange(key, request(), policy()).await }
        });
        headers_received.await.expect("request reached peer");
        state_sender.send_replace(ServiceWorkerState::Stopping);
        assert!(matches!(
            exchange.await.expect("exchange task"),
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        peer_closed_received.await.expect("peer close");
        server_task.await.expect("server task");
        drop(registry);
    }

    #[tokio::test]
    async fn closed_state_channel_cancels_active_exchange_and_closes_peer() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let vm = vm_for(key);
        let mut server = queued_pair(&vm);
        let (state_sender, state_receiver) = state();
        registry
            .register(key, vm, state_receiver)
            .expect("registration");
        let (headers_seen, headers_received) = oneshot::channel();
        let (peer_closed, peer_closed_received) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_headers(&mut server).await;
            headers_seen.send(()).expect("headers signal");
            let mut byte = [0_u8; 1];
            let result = server.read(&mut byte).await;
            assert!(matches!(result, Ok(0) | Err(_)));
            peer_closed.send(()).expect("close signal");
        });
        let exchange = tokio::spawn({
            let registry = registry.clone();
            async move { registry.exchange(key, request(), policy()).await }
        });
        headers_received.await.expect("request reached peer");
        drop(state_sender);
        assert!(matches!(
            exchange.await.expect("exchange task"),
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        peer_closed_received.await.expect("peer close");
        server_task.await.expect("server task");
        drop(registry);
    }

    #[tokio::test]
    async fn concurrent_exchanges_share_one_ready_vm() {
        let registry = GatewayServiceRegistry::new(1, 2).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let vm = vm_for(key);
        let mut first_server = queued_pair(&vm);
        let mut second_server = queued_pair(&vm);
        let (_state_sender, state_receiver) = state();
        registry
            .register(key, vm, state_receiver)
            .expect("registration");
        let response_barrier = Arc::new(Barrier::new(2));
        let first_barrier = Arc::clone(&response_barrier);
        let first_server_task = tokio::spawn(async move {
            read_headers(&mut first_server).await;
            first_barrier.wait().await;
            first_server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\none")
                .await
                .expect("first response");
        });
        let second_barrier = Arc::clone(&response_barrier);
        let second_server_task = tokio::spawn(async move {
            read_headers(&mut second_server).await;
            second_barrier.wait().await;
            second_server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ntwo")
                .await
                .expect("second response");
        });
        let first = {
            let registry = registry.clone();
            tokio::spawn(async move { registry.exchange(key, request(), policy()).await })
        };
        let second = {
            let registry = registry.clone();
            tokio::spawn(async move { registry.exchange(key, request(), policy()).await })
        };
        let (first, second) = tokio::time::timeout(Duration::from_secs(1), async {
            (
                first
                    .await
                    .expect("first exchange")
                    .expect("first response"),
                second
                    .await
                    .expect("second exchange")
                    .expect("second response"),
            )
        })
        .await
        .expect("both requests reached the service");
        assert!(
            (first.body == Bytes::from_static(b"one") && second.body == Bytes::from_static(b"two"))
                || (first.body == Bytes::from_static(b"two")
                    && second.body == Bytes::from_static(b"one"))
        );
        first_server_task.await.expect("first server");
        second_server_task.await.expect("second server");
        drop(registry);
    }

    #[tokio::test(start_paused = true)]
    async fn total_deadline_covers_delayed_open_and_http_exchange() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let key = key(Uuid::new_v4(), 1);
        let vm = TestVm::delayed(
            VmId(format!("gateway-service-{}", key.identity.instance_id)),
            Duration::from_millis(40),
        );
        let opened = vm.opened.notified();
        let mut server = queued_pair(&vm);
        let (_state_sender, state_receiver) = state();
        registry
            .register(key, vm.clone(), state_receiver)
            .expect("registration");
        let (headers_seen, headers_received) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_headers(&mut server).await;
            headers_seen.send(()).expect("headers signal");
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
        });
        let mut short = policy();
        short.exchange_timeout = Duration::from_millis(60);
        let exchange = tokio::spawn({
            let registry = registry.clone();
            async move { registry.exchange(key, request(), short).await }
        });
        opened.await;
        tokio::time::advance(Duration::from_millis(40)).await;
        headers_received.await.expect("request reached service");
        tokio::time::advance(Duration::from_millis(20)).await;
        let result = exchange.await.expect("exchange task");
        assert!(matches!(result, Err(GatewayEdgeError::HandlerUnavailable)));
        tokio::time::advance(Duration::from_millis(40)).await;
        server_task.await.expect("server task");
        drop(registry);
    }

    #[tokio::test]
    async fn unregister_keeps_global_slot_until_exchange_quiesces() {
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        let instance_key = key(Uuid::new_v4(), 1);
        let vm = vm_for(instance_key);
        let mut server = queued_pair(&vm);
        let (_state_sender, state_receiver) = state();
        registry
            .register(instance_key, vm, state_receiver)
            .expect("registration");
        let (headers_seen, headers_received) = oneshot::channel();
        let (peer_closed, peer_closed_received) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_headers(&mut server).await;
            headers_seen.send(()).expect("headers signal");
            let mut byte = [0_u8; 1];
            let result = server.read(&mut byte).await;
            assert!(matches!(result, Ok(0) | Err(_)));
            peer_closed.send(()).expect("close signal");
        });
        let exchange = tokio::spawn({
            let registry = registry.clone();
            async move { registry.exchange(instance_key, request(), policy()).await }
        });
        headers_received.await.expect("request reached peer");
        registry.unregister(instance_key).expect("unregister");
        let replacement = key(Uuid::new_v4(), 1);
        let (_replacement_sender, replacement_state) = state();
        assert_eq!(
            registry
                .register(replacement, vm_for(replacement), replacement_state,)
                .unwrap_err(),
            GatewayServiceRegistryError::CapacityExhausted
        );
        assert!(matches!(
            exchange.await.expect("exchange task"),
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        peer_closed_received.await.expect("peer close");
        server_task.await.expect("server task");
        let (_replacement_sender, replacement_state) = state();
        registry
            .register(replacement, vm_for(replacement), replacement_state)
            .expect("slot released after exchange");
        drop(registry);
    }
}
