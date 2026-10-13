//! Controlled memory VM port + actual local FD/filesystem; NOT PG/native proof.
use super::super::super::{BuildExecutionError, workspace::pin::WorkspacePin};
use super::super::{
    lock,
    models::{Mode, Outcome},
    node::Node,
    registry::{Admission, Registry},
};
use async_trait::async_trait;
pub(super) use release_domain::BuildRequestId;
pub(super) use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
pub(super) use tokio::sync::Notify;
use vm_trait::{StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvisionAttempt};

pub(super) struct Token {
    pub(super) instance: Arc<Instance>,
    pub(super) provisions: AtomicUsize,
    pub(super) cleanups: AtomicUsize,
    pub(super) provision_paused: AtomicBool,
    pub(super) provision_entered: Notify,
    pub(super) released: Notify,
    closed: AtomicBool,
}
impl Token {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            instance: Arc::new(Instance {
                id: VmId("controlled-original".into()),
                destroyed: AtomicBool::new(false),
                destroy_errors: AtomicUsize::new(0),
                paused: AtomicBool::new(false),
                entered: Notify::new(),
                released: Notify::new(),
            }),
            provisions: AtomicUsize::new(0),
            cleanups: AtomicUsize::new(0),
            provision_paused: AtomicBool::new(false),
            provision_entered: Notify::new(),
            released: Notify::new(),
            closed: AtomicBool::new(false),
        })
    }
}
#[async_trait]
impl VmProvisionAttempt for Token {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provisions.fetch_add(1, Ordering::SeqCst);
        self.provision_entered.notify_one();
        if self.provision_paused.load(Ordering::SeqCst) {
            self.released.notified().await;
        }
        if self.closed.load(Ordering::SeqCst) {
            return Err(VmError::Destroyed);
        }
        Ok(self.instance.clone())
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        self.cleanups.fetch_add(1, Ordering::SeqCst);
        self.closed.store(true, Ordering::SeqCst);
        self.released.notify_one();
        self.instance.destroy().await
    }
}
pub(super) struct Instance {
    id: VmId,
    pub(super) destroyed: AtomicBool,
    pub(super) destroy_errors: AtomicUsize,
    pub(super) paused: AtomicBool,
    pub(super) entered: Notify,
    pub(super) released: Notify,
}
#[async_trait]
impl VmInstance for Instance {
    fn id(&self) -> &VmId {
        &self.id
    }
    async fn start(&self) -> Result<(), VmError> {
        Err(VmError::InvalidState("controlled Start failure"))
    }
    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        self.destroy().await
    }
    async fn wait(&self) -> Result<VmExit, VmError> {
        Err(VmError::Destroyed)
    }
    fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VmEvent> {
        tokio::sync::broadcast::channel(4).1
    }
    async fn destroy(&self) -> Result<(), VmError> {
        self.entered.notify_one();
        if self
            .destroy_errors
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(VmError::InvalidState("controlled unresolved detach"));
        }
        if self.paused.load(Ordering::SeqCst) {
            self.released.notified().await;
        }
        self.destroyed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
pub(super) struct Fixture {
    pub(super) temporary: tempfile::TempDir,
    pub(super) registry: Registry,
    pub(super) node: Arc<Node>,
    pub(super) token: Arc<Token>,
    pub(super) pin: Arc<WorkspacePin>,
}
impl Fixture {
    pub(super) fn new(mode: Mode) -> Self {
        use std::{fs, os::unix::fs::PermissionsExt};
        let temporary = tempfile::tempdir().unwrap();
        let parent = WorkspacePin::open(temporary.path(), None).unwrap();
        let path = temporary.path().join("original");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(path.join("output"), b"original sealed output").unwrap();
        let pin = WorkspacePin::open(&path, Some(parent)).unwrap();
        let registry = Registry::default();
        let Admission::Start(node, _) = registry.register(BuildRequestId::new(), mode).unwrap()
        else {
            panic!("new original")
        };
        node.workspace(pin.clone());
        Self {
            temporary,
            registry,
            node,
            token: Token::new(),
            pin,
        }
    }
    pub(super) fn path(&self) -> std::path::PathBuf {
        self.temporary.path().join("original")
    }
}
pub(super) async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .expect("bounded controlled owner")
}
pub(super) async fn watcher_finished(node: &Arc<Node>) {
    bounded(async {
        loop {
            node.join_finished_jobs().await;
            if lock(&node.execution_job)
                .as_ref()
                .is_some_and(|job| job.joined())
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}
pub(super) fn failure() -> Outcome {
    Outcome::Build(Err(Arc::new(BuildExecutionError::Vm)))
}
