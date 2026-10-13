use super::super::{Node, Owner, pin::Pin};
use async_trait::async_trait;
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
    pub(super) attempts: AtomicUsize,
    pub(super) cleanups: AtomicUsize,
    pub(super) entered: Notify,
    pub(super) release: Notify,
    pub(super) fail: AtomicUsize,
    pub(super) paused: AtomicBool,
    closed: AtomicBool,
}
impl Token {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            instance: Arc::new(Instance {
                destroyed: AtomicBool::new(false),
                id: VmId("controlled-oci-original".into()),
            }),
            attempts: AtomicUsize::new(0),
            cleanups: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            fail: AtomicUsize::new(0),
            paused: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        })
    }
}
#[async_trait]
impl VmProvisionAttempt for Token {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.paused.load(Ordering::SeqCst) {
            self.release.notified().await;
        }
        if self.closed.load(Ordering::SeqCst) {
            Err(VmError::Destroyed)
        } else {
            Ok(self.instance.clone())
        }
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        self.cleanups.fetch_add(1, Ordering::SeqCst);
        if self
            .fail
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(VmError::InvalidState(
                "controlled unresolved original detach",
            ));
        }
        self.closed.store(true, Ordering::SeqCst);
        self.release.notify_one();
        self.instance.destroy().await
    }
}
pub(super) struct Instance {
    pub(super) destroyed: AtomicBool,
    id: VmId,
}
#[async_trait]
impl VmInstance for Instance {
    fn id(&self) -> &VmId {
        &self.id
    }
    async fn start(&self) -> Result<(), VmError> {
        Err(VmError::InvalidState("controlled OCI Start failure"))
    }
    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        self.destroy().await
    }
    async fn wait(&self) -> Result<VmExit, VmError> {
        Err(VmError::Destroyed)
    }
    async fn destroy(&self) -> Result<(), VmError> {
        self.destroyed.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VmEvent> {
        tokio::sync::broadcast::channel(4).1
    }
}
pub(super) struct Fixture {
    pub(super) temporary: tempfile::TempDir,
    pub(super) owner: Arc<Owner>,
    pub(super) node: Arc<Node>,
    pub(super) token: Arc<Token>,
}
impl Fixture {
    pub(super) fn new() -> Self {
        use std::{fs, os::unix::fs::PermissionsExt};
        let temporary = tempfile::tempdir().unwrap();
        let roots =
            ["candidate", "scratch", "verification"].map(|name| temporary.path().join(name));
        for root in &roots {
            fs::create_dir(root).unwrap();
            fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let program = fs::canonicalize("/bin/true").unwrap();
        let owner = Arc::new(Owner::initialize(&roots[0], &roots[1], &roots[2], &program).unwrap());
        let request = input(temporary.path());
        let id = request.job_id;
        let paths = [
            roots[1].join(format!("{id}.raw")),
            roots[0].join(id.to_string()),
            roots[2].join(id.to_string()),
        ];
        let (node, new, matched) = owner.registry.admit(request, paths).unwrap();
        assert!(new && matched);
        Self {
            temporary,
            owner,
            node,
            token: Token::new(),
        }
    }
    pub(super) fn scratch(&self) {
        use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};
        self.node.acquire(0).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.node.paths[0])
            .unwrap();
        let file = self.node.capture_fd(0, file);
        let pin =
            Pin::scratch(file, &self.node.paths[0], self.owner.roots.scratch.clone()).unwrap();
        self.node.leaf(0, pin);
    }
    pub(super) fn spec(&self) -> vm_trait::VmSpec {
        let scratch = crate::constants::ScratchDisk {
            path: self.node.paths[0].clone(),
            filesystem_uuid: uuid::Uuid::new_v4(),
        };
        let mut spec = crate::specs::builder_vm_spec(
            &self.node.request,
            &self.node.paths[1],
            &scratch,
            &vm_trait::RootFilesystem::Directory {
                host_path: self.temporary.path().to_path_buf(),
            },
            &vm_trait::VmResources {
                vcpus: 1,
                memory_mib: 64,
            },
        )
        .unwrap();
        spec.id = self.token.instance.id().clone();
        spec
    }
}
fn input(root: &std::path::Path) -> oci_builder_worker::IsolatedOciBuild {
    oci_builder_worker::IsolatedOciBuild {
        job_id: uuid::Uuid::new_v4(),
        image_id: builder_catalog_domain::OciImageId::from_uuid(uuid::Uuid::new_v4()),
        project_id: uuid::Uuid::new_v4(),
        dockerfile: root.join("Dockerfile"),
        checkout_root: root.to_path_buf(),
        context: root.to_path_buf(),
        base_oci_layout: root.join("base"),
        base_reference: builder_catalog_domain::OciImageReference::parse(format!(
            "registry.example/platform/images/ubuntu-native@sha256:{}",
            "a".repeat(64)
        ))
        .unwrap(),
        network_disabled: true,
        ambient_credentials_disabled: true,
    }
}
pub(super) async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .expect("bounded controlled original owner")
}
pub(super) fn failed() -> super::super::models::Outcome {
    Err(Arc::new(oci_builder_worker::OciWorkerError::BuildFailed))
}
pub(super) async fn ready(node: &Arc<Node>) {
    bounded(async {
        loop {
            node.join_finished().await;
            if node.delivery_joined() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}
