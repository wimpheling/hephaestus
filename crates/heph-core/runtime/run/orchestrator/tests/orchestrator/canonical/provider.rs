use async_trait::async_trait;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::broadcast;
use vm_trait::{
    StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec,
};

use super::super::support::{AutoExitProvider, lock};

pub struct ScopedProvider {
    pub inner: AutoExitProvider,
    pub scope: Mutex<VmProviderOwnerScope>,
    pub fail_cleanup: AtomicBool,
    pub cleanup_delay_seconds: AtomicU64,
    pub hang_cleanup: AtomicBool,
    pub fail_destroy: Arc<AtomicBool>,
    pub wrong_handle: AtomicBool,
    pub pause_provision: Mutex<Option<Arc<Pause>>>,
    pub pause_start: Mutex<Option<Arc<Pause>>>,
    pub instances: Mutex<Vec<Weak<dyn VmInstance>>>,
}

pub struct Pause {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}

impl Pause {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }
    pub async fn wait(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}

impl ScopedProvider {
    pub fn new(log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            inner: AutoExitProvider::new(log),
            scope: Mutex::new(
                VmProviderOwnerScope::new("test-owner".into(), "test".into()).unwrap(),
            ),
            fail_cleanup: AtomicBool::new(false),
            cleanup_delay_seconds: AtomicU64::new(0),
            hang_cleanup: AtomicBool::new(false),
            fail_destroy: Arc::new(AtomicBool::new(false)),
            wrong_handle: AtomicBool::new(false),
            pause_provision: Mutex::new(None),
            pause_start: Mutex::new(None),
            instances: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl VmProvider for ScopedProvider {
    fn name(&self) -> &'static str {
        "scoped-test"
    }

    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        Ok(lock(&self.scope).clone())
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let pause = lock(&self.pause_provision).clone();
        if let Some(pause) = pause {
            pause.wait().await;
        }
        let id = if self.wrong_handle.load(Ordering::SeqCst) {
            VmId("different-vm".into())
        } else {
            spec.id.clone()
        };
        let instance: Arc<dyn VmInstance> = Arc::new(ScopedInstance {
            inner: self.inner.provision(spec).await?,
            id,
            fail_destroy: Arc::clone(&self.fail_destroy),
            pause_start: lock(&self.pause_start).clone(),
        });
        lock(&self.instances).push(Arc::downgrade(&instance));
        Ok(instance)
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState(
            "unscoped cleanup must never be called",
        ))
    }

    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        _id: &VmId,
    ) -> Result<(), VmError> {
        if scope != &*lock(&self.scope) {
            return Err(VmError::InvalidState("provider ownership changed"));
        }
        lock(&self.inner.log).push("scoped-cleanup");
        let delay = self.cleanup_delay_seconds.load(Ordering::SeqCst);
        if delay != 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        if self.hang_cleanup.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        if self.fail_cleanup.load(Ordering::SeqCst) {
            return Err(VmError::InvalidState("physical cleanup uncertain"));
        }
        Ok(())
    }
}

struct ScopedInstance {
    inner: Arc<dyn VmInstance>,
    id: VmId,
    fail_destroy: Arc<AtomicBool>,
    pause_start: Option<Arc<Pause>>,
}

#[async_trait]
impl VmInstance for ScopedInstance {
    fn id(&self) -> &VmId {
        &self.id
    }
    async fn start(&self) -> Result<(), VmError> {
        if let Some(pause) = self.pause_start.as_ref() {
            pause.wait().await;
        }
        self.inner.start().await
    }
    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        self.inner.stop(mode).await
    }
    async fn wait(&self) -> Result<VmExit, VmError> {
        self.inner.wait().await
    }
    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.inner.subscribe_events()
    }
    async fn destroy(&self) -> Result<(), VmError> {
        if self.fail_destroy.load(Ordering::SeqCst) {
            return Err(VmError::InvalidState("active VM destroy failed"));
        }
        self.inner.destroy().await
    }
}
