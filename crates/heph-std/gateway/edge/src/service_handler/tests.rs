use super::*;
use crate::{
    GatewayExecutionTargetError, GatewayLimits, GatewayScheme, GatewayServiceAuthorityBudget,
    GatewayServiceIdentity, GatewayServiceInstanceKey, ServiceWorkerState, TrustedRequestMetadata,
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
        exposure: gateway_domain::Exposure::Public,
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

fn budget(key: GatewayServiceInstanceKey, remaining: Duration) -> GatewayServiceAuthorityBudget {
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

#[path = "handler_scenarios.rs"]
mod handler_scenarios;
