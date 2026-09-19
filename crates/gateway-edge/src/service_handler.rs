//! Routes accepted gateway invocations to stateless handlers or warm services.

use async_trait::async_trait;
use tokio::time::{self, Instant};
use uuid::Uuid;

use crate::{
    GatewayEdgeError, GatewayExecutionTarget, GatewayExecutionTargetResolver, GatewayRequest,
    GatewayResponse, GatewayRouteBinding, GatewayServiceOwner, GatewayServiceRegistry,
    GatewayVmHandler, ServiceHttpPolicy,
};

/// Dispatches an already accepted invocation to its immutable execution target.
pub struct GatewayServiceHandler<R, H> {
    resolver: R,
    stateless: H,
    registry: GatewayServiceRegistry,
    owner: GatewayServiceOwner,
}

impl<R, H> GatewayServiceHandler<R, H> {
    /// Creates a dispatcher with one local service owner and warm-instance
    /// registry.
    ///
    /// # Errors
    ///
    /// Returns an invalid-route error when the configured owner is malformed.
    pub fn new(
        resolver: R,
        stateless: H,
        registry: GatewayServiceRegistry,
        owner: GatewayServiceOwner,
    ) -> Result<Self, GatewayEdgeError> {
        owner
            .validate()
            .map_err(|_| GatewayEdgeError::InvalidRoute("invalid service owner"))?;
        Ok(Self {
            resolver,
            stateless,
            registry,
            owner,
        })
    }
}

#[async_trait]
impl<R, H> GatewayVmHandler for GatewayServiceHandler<R, H>
where
    R: GatewayExecutionTargetResolver,
    H: GatewayVmHandler,
{
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        let route_deadline = Instant::now()
            .checked_add(route.limits.execution_timeout)
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let target = time::timeout_at(
            route_deadline,
            self.resolver.resolve_execution_target(
                invocation_id,
                route.route_id,
                route.gateway_revision_id,
                &self.owner,
            ),
        )
        .await
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;

        match target {
            GatewayExecutionTarget::Stateless => {
                let response = time::timeout_at(
                    route_deadline,
                    self.stateless.invoke(route, invocation_id, request),
                )
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)??;
                Ok(response)
            }
            GatewayExecutionTarget::Service(budget) => {
                let authority_deadline = Instant::now()
                    .checked_add(budget.remaining)
                    .ok_or(GatewayEdgeError::HandlerUnavailable)?
                    .min(route_deadline);
                let exchange_timeout = authority_deadline.saturating_duration_since(Instant::now());
                if exchange_timeout.is_zero() {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                let policy = ServiceHttpPolicy::from_gateway_limits(route.limits);
                let policy = ServiceHttpPolicy {
                    exchange_timeout,
                    ..policy
                };
                time::timeout_at(
                    authority_deadline,
                    self.registry.exchange(budget.instance, request, policy),
                )
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GatewayExecutionTargetError, GatewayLimits, GatewayScheme, GatewayServiceAuthorityBudget,
        GatewayServiceIdentity, GatewayServiceInstanceKey, ServiceWorkerState,
        TrustedRequestMetadata,
    };
    use ::time::OffsetDateTime;
    use async_trait::async_trait;
    use bytes::Bytes;
    use http::{HeaderMap, HeaderValue, Method, StatusCode};
    use std::{
        collections::{BTreeSet, VecDeque},
        net::IpAddr,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{broadcast, oneshot, watch},
    };
    use uuid::Uuid;
    use vm_trait::{
        BoxedPrivateServiceConnection, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance,
    };

    struct Resolver {
        target: Result<GatewayExecutionTarget, GatewayExecutionTargetError>,
        delay: Duration,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl GatewayExecutionTargetResolver for Resolver {
        async fn resolve_execution_target(
            &self,
            _: Uuid,
            _: Uuid,
            _: Uuid,
            _: &GatewayServiceOwner,
        ) -> Result<GatewayExecutionTarget, GatewayExecutionTargetError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(self.delay).await;
            self.target.clone()
        }
    }

    struct Stateless {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl GatewayVmHandler for Stateless {
        async fn invoke(
            &self,
            _: &GatewayRouteBinding,
            _: Uuid,
            _: GatewayRequest,
        ) -> Result<GatewayResponse, GatewayEdgeError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(response(StatusCode::NO_CONTENT, Bytes::new()))
        }
    }

    struct TestVm {
        id: VmId,
        connections: Mutex<VecDeque<BoxedPrivateServiceConnection>>,
        events: broadcast::Sender<VmEvent>,
        _state_sender: watch::Sender<ServiceWorkerState>,
    }

    impl TestVm {
        fn new(
            id: VmId,
            connection: BoxedPrivateServiceConnection,
            state_sender: watch::Sender<ServiceWorkerState>,
        ) -> Arc<Self> {
            let (events, _) = broadcast::channel(2);
            Arc::new(Self {
                id,
                connections: Mutex::new(VecDeque::from([connection])),
                events,
                _state_sender: state_sender,
            })
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
            Err(VmError::InvalidState("test VM does not exit"))
        }

        async fn open_private_service_connection(
            &self,
        ) -> Result<BoxedPrivateServiceConnection, VmError> {
            self.connections
                .lock()
                .expect("connection mutex")
                .pop_front()
                .ok_or(VmError::InvalidState("test connection already used"))
        }

        fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
            self.events.subscribe()
        }

        async fn destroy(&self) -> Result<(), VmError> {
            Ok(())
        }
    }

    fn route(timeout: Duration) -> GatewayRouteBinding {
        GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: "example".to_owned(),
            methods: BTreeSet::from([Method::GET]),
            limits: GatewayLimits {
                max_request_body_bytes: 1024,
                max_response_body_bytes: 1024,
                max_request_headers: 16,
                max_response_headers: 16,
                max_path_and_query_bytes: 256,
                execution_timeout: timeout,
            },
        }
    }

    fn request() -> GatewayRequest {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer rewritten-application-secret"),
        );
        GatewayRequest {
            method: Method::GET,
            path_and_query: "/healthz?check=1".to_owned(),
            headers,
            body: Bytes::new(),
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Https,
                authority: "public.example".to_owned(),
                client_address: "127.0.0.1".parse::<IpAddr>().expect("address"),
                request_id: Uuid::new_v4(),
            },
        }
    }

    fn response(status: StatusCode, body: Bytes) -> GatewayResponse {
        GatewayResponse {
            status,
            headers: HeaderMap::new(),
            body,
            mailbox_publication: None,
        }
    }

    fn owner() -> GatewayServiceOwner {
        GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner")
    }

    fn budget(
        key: GatewayServiceInstanceKey,
        remaining: Duration,
    ) -> GatewayServiceAuthorityBudget {
        GatewayServiceAuthorityBudget {
            instance: key,
            expires_at: OffsetDateTime::now_utc() + remaining,
            remaining,
        }
    }

    fn key() -> GatewayServiceInstanceKey {
        GatewayServiceInstanceKey {
            identity: GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                gateway_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            },
            fencing_token: 7,
        }
    }

    fn service_fixture(
        response_bytes: &'static [u8],
    ) -> (
        GatewayServiceRegistry,
        GatewayServiceInstanceKey,
        oneshot::Receiver<Vec<u8>>,
    ) {
        service_fixture_with_delay(response_bytes, Duration::ZERO)
    }

    fn service_fixture_with_delay(
        response_bytes: &'static [u8],
        response_delay: Duration,
    ) -> (
        GatewayServiceRegistry,
        GatewayServiceInstanceKey,
        oneshot::Receiver<Vec<u8>>,
    ) {
        let key = key();
        let (host, mut peer) = tokio::io::duplex(4096);
        let (captured, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 512];
            loop {
                let count = peer.read(&mut buffer).await.expect("request read");
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = captured.send(bytes);
            tokio::time::sleep(response_delay).await;
            peer.write_all(response_bytes)
                .await
                .expect("response write");
            peer.shutdown().await.expect("response shutdown");
        });
        let (state_sender, state) = watch::channel(ServiceWorkerState::Ready);
        let vm = TestVm::new(
            VmId(format!("gateway-service-{}", key.identity.instance_id)),
            Box::new(host),
            state_sender,
        );
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        registry.register(key, vm, state).expect("register");
        (registry, key, receiver)
    }

    #[tokio::test]
    async fn stateless_target_delegates_without_service_exchange() {
        let route = route(Duration::from_secs(1));
        let stateless = Stateless {
            calls: AtomicUsize::new(0),
        };
        let resolver = Resolver {
            target: Ok(GatewayExecutionTarget::Stateless),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        };
        let handler = GatewayServiceHandler::new(
            resolver,
            stateless,
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            owner(),
        )
        .expect("handler");
        let response = handler
            .invoke(&route, Uuid::new_v4(), request())
            .await
            .expect("stateless response");
        assert_eq!(response.status, StatusCode::NO_CONTENT);
        assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 1);
        drop(handler);
    }

    #[tokio::test]
    async fn service_target_uses_exact_registry_key_and_preserves_application_headers() {
        let route = route(Duration::from_secs(1));
        let (registry, key, captured) =
            service_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let stateless = Stateless {
            calls: AtomicUsize::new(0),
        };
        let resolver = Resolver {
            target: Ok(GatewayExecutionTarget::Service(budget(
                key,
                Duration::from_secs(1),
            ))),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        };
        let handler =
            GatewayServiceHandler::new(resolver, stateless, registry, owner()).expect("handler");
        let mut inbound = request();
        inbound
            .headers
            .insert("x-application", HeaderValue::from_static("kept"));
        let response = handler
            .invoke(&route, Uuid::new_v4(), inbound)
            .await
            .expect("service response");
        assert_eq!(response.status, StatusCode::OK);
        let wire = captured.await.expect("captured request");
        let wire = String::from_utf8(wire).expect("HTTP bytes");
        assert!(wire.contains("x-application: kept"));
        assert!(wire.contains("authorization: Bearer rewritten-application-secret"));
        assert_eq!(wire.matches("authorization:").count(), 1);
        assert!(!wire.to_ascii_lowercase().contains("x-platform"));
        assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 0);
        drop(handler);
    }

    #[tokio::test]
    async fn missing_or_stale_service_target_does_not_fall_back() {
        let route = route(Duration::from_secs(1));
        let (registry, registered_key, _captured) =
            service_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let stale_key = GatewayServiceInstanceKey {
            identity: registered_key.identity,
            fencing_token: registered_key.fencing_token + 1,
        };
        let stateless = Stateless {
            calls: AtomicUsize::new(0),
        };
        let resolver = Resolver {
            target: Ok(GatewayExecutionTarget::Service(budget(
                stale_key,
                Duration::from_secs(1),
            ))),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        };
        let handler =
            GatewayServiceHandler::new(resolver, stateless, registry, owner()).expect("handler");
        let result = handler.invoke(&route, Uuid::new_v4(), request()).await;
        assert!(result.is_err());
        assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 0);
        drop(handler);
    }

    #[tokio::test]
    async fn resolver_failure_does_not_fall_back() {
        let route = route(Duration::from_secs(1));
        let stateless = Stateless {
            calls: AtomicUsize::new(0),
        };
        let handler = GatewayServiceHandler::new(
            Resolver {
                target: Err(GatewayExecutionTargetError::Unavailable),
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            },
            stateless,
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            owner(),
        )
        .expect("handler");
        assert!(
            handler
                .invoke(&route, Uuid::new_v4(), request())
                .await
                .is_err()
        );
        assert_eq!(handler.stateless.calls.load(Ordering::Relaxed), 0);
        drop(handler);
    }

    #[test]
    fn constructor_rejects_invalid_owner() {
        let invalid = GatewayServiceOwner {
            host_id: String::new(),
            owner_uuid: Uuid::new_v4(),
        };
        let result = GatewayServiceHandler::new(
            Resolver {
                target: Ok(GatewayExecutionTarget::Stateless),
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            },
            Stateless {
                calls: AtomicUsize::new(0),
            },
            GatewayServiceRegistry::new(1, 1).expect("registry"),
            invalid,
        );
        assert!(matches!(
            result,
            Err(GatewayEdgeError::InvalidRoute("invalid service owner"))
        ));
        drop(result);
    }

    #[tokio::test(start_paused = true)]
    async fn lookup_and_exchange_share_one_route_deadline() {
        let route = route(Duration::from_millis(50));
        let (registry, key, _captured) = service_fixture_with_delay(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            Duration::from_millis(30),
        );
        let stateless = Stateless {
            calls: AtomicUsize::new(0),
        };
        let resolver = Resolver {
            target: Ok(GatewayExecutionTarget::Service(budget(
                key,
                Duration::from_secs(1),
            ))),
            delay: Duration::from_millis(30),
            calls: AtomicUsize::new(0),
        };
        let handler =
            GatewayServiceHandler::new(resolver, stateless, registry, owner()).expect("handler");
        let invocation =
            tokio::spawn(async move { handler.invoke(&route, Uuid::new_v4(), request()).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(30)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(20)).await;
        assert!(invocation.await.expect("lookup task").is_err());
    }

    #[tokio::test]
    // The registry is deliberately moved into the handler and retained until
    // the cancellation stream has observed its exchange cleanup.
    #[allow(clippy::significant_drop_tightening)]
    async fn session_budget_cancellation_closes_service_exchange() {
        let route = route(Duration::from_secs(1));
        let key = key();
        let (host, mut peer) = tokio::io::duplex(4096);
        let (closed, closed_receiver) = oneshot::channel();
        tokio::spawn(async move {
            let mut bytes = [0_u8; 512];
            let _ = peer.read(&mut bytes).await;
            let mut rest = Vec::new();
            let _ = peer.read_to_end(&mut rest).await;
            let _ = closed.send(());
        });
        let (state_sender, state) = watch::channel(ServiceWorkerState::Ready);
        let vm = TestVm::new(
            VmId(format!("gateway-service-{}", key.identity.instance_id)),
            Box::new(host),
            state_sender,
        );
        let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
        registry.register(key, vm, state).expect("register");
        let handler = GatewayServiceHandler::new(
            Resolver {
                target: Ok(GatewayExecutionTarget::Service(budget(
                    key,
                    Duration::from_millis(25),
                ))),
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            },
            Stateless {
                calls: AtomicUsize::new(0),
            },
            registry,
            owner(),
        )
        .expect("handler");
        let result = handler.invoke(&route, Uuid::new_v4(), request()).await;
        assert!(result.is_err());
        tokio::time::timeout(Duration::from_secs(1), closed_receiver)
            .await
            .expect("service stream closed")
            .expect("close marker");
        drop(handler);
    }
}
