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

#[path = "service_registry_tests/exchange_lifecycle.rs"]
mod exchange_lifecycle;
#[path = "service_registry_tests/registration.rs"]
mod registration;
