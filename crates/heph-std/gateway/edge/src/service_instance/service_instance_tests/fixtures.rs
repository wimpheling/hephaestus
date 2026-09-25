use crate::{
    GatewayEdgeError, GatewayServiceIdentity, GatewayServiceLaunch, GatewayServiceLaunchResolver,
    ServiceInstancePolicy,
};
use async_trait::async_trait;
use gateway_domain::ServiceProbePath;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Notify, broadcast},
};
use uuid::Uuid;
use vm_trait::{
    BoxedPrivateServiceConnection, GuestCommand, NetworkMode, RootFilesystem, StopMode, VmError,
    VmExit, VmId, VmInstance, VmResources, VmSpec,
};
pub(super) struct FakeVm {
    pub(super) id: VmId,
    pub(super) connections: Mutex<VecDeque<BoxedPrivateServiceConnection>>,
    pub(super) events: broadcast::Sender<vm_trait::VmEvent>,
    pub(super) exited: tokio::sync::watch::Sender<Option<VmExit>>,
    pub(super) first_open: Arc<Notify>,
    pub(super) starts: AtomicUsize,
    pub(super) opens: AtomicUsize,
    pub(super) destroys: AtomicUsize,
    pub(super) destroy_ok: AtomicBool,
    pub(super) start_ok: AtomicBool,
    pub(super) emit_start_log: AtomicBool,
    pub(super) stop_ok: AtomicBool,
    pub(super) start_delay: Mutex<Duration>,
}

impl FakeVm {
    pub(super) fn new(id: VmId) -> Arc<Self> {
        let (events, _) = broadcast::channel(2);
        let (exited, _) = tokio::sync::watch::channel(None);
        Arc::new(Self {
            id,
            connections: Mutex::new(VecDeque::new()),
            events,
            exited,
            first_open: Arc::new(Notify::new()),
            starts: AtomicUsize::new(0),
            opens: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
            destroy_ok: AtomicBool::new(true),
            start_ok: AtomicBool::new(true),
            emit_start_log: AtomicBool::new(false),
            stop_ok: AtomicBool::new(true),
            start_delay: Mutex::new(Duration::ZERO),
        })
    }

    pub(super) fn push(&self, connection: BoxedPrivateServiceConnection) {
        self.connections
            .lock()
            .expect("connection lock")
            .push_back(connection);
    }

    pub(super) fn exit_with(&self, exit: VmExit) {
        let _ = self.exited.send(Some(exit));
    }
}

#[async_trait]
impl VmInstance for FakeVm {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        if !self.start_ok.load(Ordering::Relaxed) {
            return Err(VmError::Destroyed);
        }
        if self.emit_start_log.load(Ordering::Relaxed) {
            let _ = self.events.send(vm_trait::VmEvent::Log {
                stream: vm_trait::LogStream::Stdout,
                bytes: b"before-readiness".to_vec(),
            });
        }
        let _ = self.events.send(vm_trait::VmEvent::Started {
            ingress: Vec::new(),
        });
        let delay = *self.start_delay.lock().expect("start delay lock");
        tokio::time::sleep(delay).await;
        Ok(())
    }

    async fn stop(&self, _: StopMode) -> Result<(), VmError> {
        if self.stop_ok.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err(VmError::Destroyed)
        }
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exited.subscribe();
        loop {
            let current = receiver.borrow().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver.changed().await.map_err(|_| VmError::Destroyed)?;
        }
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        if self.opens.fetch_add(1, Ordering::Relaxed) == 0 {
            self.first_open.notify_waiters();
        }
        self.connections
            .lock()
            .expect("connection lock")
            .pop_front()
            .ok_or_else(|| VmError::Unavailable {
                resource: String::from("fake connection"),
                reason: String::from("none queued"),
            })
    }

    fn subscribe_events(&self) -> broadcast::Receiver<vm_trait::VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        self.destroys.fetch_add(1, Ordering::Relaxed);
        if self.destroy_ok.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err(VmError::Destroyed)
        }
    }
}

pub(super) struct FakeResolver {
    pub(super) cleanups: AtomicUsize,
}

#[async_trait]
impl GatewayServiceLaunchResolver for FakeResolver {
    async fn resolve_service_launch(
        &self,
        _: crate::GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }

    async fn cleanup_service_launch(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

pub(super) fn launch(instance_id: Uuid) -> GatewayServiceLaunch {
    let identity = GatewayServiceIdentity {
        instance_id,
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let ready = ServiceProbePath::parse("/readyz").expect("ready path");
    let health = ServiceProbePath::parse("/healthz").expect("health path");
    GatewayServiceLaunch {
        identity,
        service: gateway_domain::GatewayServiceConfig::new(8080, ready, health).expect("service"),
        spec: VmSpec {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/fake/root"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 64,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/service"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels: BTreeMap::new(),
        },
    }
}

pub(super) fn launch_with_logs(instance_id: Uuid) -> GatewayServiceLaunch {
    let mut launch = launch(instance_id);
    launch.service = launch
        .service
        .with_log_capture_mode(gateway_domain::ServiceLogCaptureMode::Application);
    launch
}

pub(super) fn policy() -> ServiceInstancePolicy {
    ServiceInstancePolicy::new(
        Duration::from_millis(120),
        Duration::from_millis(1),
        Duration::from_millis(30),
        Duration::from_millis(50),
    )
}

pub(super) async fn response_peer(mut peer: DuplexStream, status: u16) {
    let mut request = [0_u8; 1024];
    let _ = peer.read(&mut request).await;
    let response =
        format!("HTTP/1.1 {status} OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    let _ = peer.write_all(response.as_bytes()).await;
}
