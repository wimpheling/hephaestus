use super::helpers::{ProcessStatus, wire_to_vm_error};

pub(super) use crate::{
    cgroup::Cgroup,
    config::LibkrunConfig,
    framing::{read_async, write_async},
    protocol::{PrivateServiceConnectionMessage, SUPERVISOR_SOCKET_NAME},
    service_transport::ServiceBroker,
    validation::{
        PROVIDER_NAME, PreparedForward, PreparedSpec, prepare_spec, validate_config, validate_id,
    },
    worker::{
        WireError, WireErrorKind, WireLogStream, WorkerCommand, WorkerConfiguration, WorkerEvent,
        WorkerMessage, WorkerRequest,
    },
};
pub(super) use async_trait::async_trait;
pub(super) use bytes::Bytes;
pub(super) use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
pub(super) use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt, fs, io,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
pub(super) use tokio::{
    net::{unix::OwnedReadHalf, unix::OwnedWriteHalf},
    process::{Child, Command},
    sync::{Mutex, Semaphore, broadcast, oneshot, watch},
    time::{sleep, timeout},
};
pub(super) use tracing::{error, info, warn};
pub(super) use vm_trait::{
    BoxedPrivateServiceConnection, LogStream, PortForward, PortProtocol, StopMode, VmError,
    VmEvent, VmExit, VmId, VmInstance, VmMetric, VmProvider, VmSpec,
};

pub(super) const EVENT_CAPACITY: usize = 256;
pub(super) const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);
pub(super) const PRIVATE_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Fedora/Linux VM provider backed by a dedicated libkrun worker per VM.
pub(super) struct ProviderInner {
    pub(super) config: Arc<LibkrunConfig>,
    pub(super) ids: Mutex<HashSet<VmId>>,
    pub(super) worker_spawner: Arc<dyn WorkerSpawner>,
}
pub(super) struct LibkrunInstance {
    pub(super) id: VmId,
    pub(super) config: Arc<LibkrunConfig>,
    pub(super) worker: Arc<dyn WorkerBackend>,
    pub(super) state: Mutex<Lifecycle>,
    pub(super) terminal: watch::Sender<Option<Terminal>>,
    pub(super) terminal_guard: Mutex<()>,
    pub(super) ready: watch::Sender<bool>,
    pub(super) start_result: watch::Sender<Option<Result<(), ErrorSnapshot>>>,
    pub(super) events: broadcast::Sender<VmEvent>,
    pub(super) resources: Mutex<Option<OwnedResources>>,
    pub(super) provider_ids: Arc<ProviderInner>,
    pub(super) private_http_enabled: bool,
    pub(super) private_service_timeout: Option<Duration>,
    pub(super) private_service_dispatch: Option<Arc<Semaphore>>,
    pub(super) private_http_waiters:
        Mutex<HashMap<u64, oneshot::Sender<Result<vm_trait::PrivateHttpResponse, VmError>>>>,
    pub(super) next_private_http_request: AtomicU64,
}

#[derive(Clone)]
pub(super) enum Terminal {
    Exited(VmExit),
    Destroyed,
}

pub(super) enum Lifecycle {
    Provisioned,
    Starting,
    Running,
    Stopping,
    Exited,
    StartFailed(ErrorSnapshot),
    Destroyed,
}

pub(super) struct OwnedResources {
    pub(super) runtime_dir: PathBuf,
    pub(super) cgroup: Cgroup,
    pub(super) service_broker: Option<Arc<ServiceBroker>>,
}

#[derive(Debug, Clone)]
pub(super) struct ErrorSnapshot {
    kind: WireErrorKind,
    code: String,
    message: String,
}

impl ErrorSnapshot {
    pub(super) fn from_vm_error(error: &VmError) -> Self {
        match error {
            VmError::InvalidSpec { field, reason } => Self {
                kind: WireErrorKind::InvalidSpec,
                code: field.clone(),
                message: reason.clone(),
            },
            VmError::Unsupported { feature, .. } => Self {
                kind: WireErrorKind::Unsupported,
                code: "unsupported".to_owned(),
                message: feature.clone(),
            },
            VmError::Unavailable { resource, reason } => Self {
                kind: WireErrorKind::Unavailable,
                code: resource.clone(),
                message: reason.clone(),
            },
            VmError::InvalidState(message) => Self {
                kind: WireErrorKind::InvalidState,
                code: "invalid-state".to_owned(),
                message: (*message).to_owned(),
            },
            VmError::Destroyed => Self {
                kind: WireErrorKind::Destroyed,
                code: "destroyed".to_owned(),
                message: error.to_string(),
            },
            VmError::Provider { code, .. } => Self {
                kind: WireErrorKind::Backend,
                code: code.clone(),
                message: error.to_string(),
            },
            _ => Self {
                kind: WireErrorKind::Backend,
                code: "start".to_owned(),
                message: error.to_string(),
            },
        }
    }

    pub(super) fn to_vm_error(&self) -> VmError {
        wire_to_vm_error(WireError {
            kind: self.kind,
            code: self.code.clone(),
            message: self.message.clone(),
        })
    }
}

pub(super) type PendingResponse = oneshot::Sender<Result<(), WireError>>;
pub(super) type PendingRequests = Arc<Mutex<HashMap<u64, PendingResponse>>>;

pub(super) struct WorkerClient {
    pub(super) writer: Mutex<OwnedWriteHalf>,
    pub(super) pending: PendingRequests,
    pub(super) next_request: AtomicU64,
    pub(super) events: broadcast::Sender<WorkerEvent>,
    pub(super) child: Arc<Mutex<Child>>,
    pub(super) process_exit: watch::Sender<Option<ProcessStatus>>,
    pub(super) pid: u32,
}

#[async_trait]
pub(super) trait WorkerBackend: Send + Sync {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError>;
    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent>;
    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>>;
    async fn kill(&self) -> Result<(), VmError>;
    async fn wait_process(&self) -> Result<ProcessStatus, VmError>;
}

#[async_trait]
pub(super) trait WorkerSpawner: Send + Sync {
    async fn spawn(
        &self,
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
    ) -> Result<Arc<dyn WorkerBackend>, VmError>;
}

pub(super) struct ProcessWorkerSpawner;

#[async_trait]
impl WorkerSpawner for ProcessWorkerSpawner {
    async fn spawn(
        &self,
        config: Arc<LibkrunConfig>,
        spec: PreparedSpec,
        runtime_dir: &Path,
        cgroup: &Cgroup,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        WorkerClient::launch(config, spec, runtime_dir, cgroup)
            .await
            .map(|worker| worker as Arc<dyn WorkerBackend>)
    }
}
