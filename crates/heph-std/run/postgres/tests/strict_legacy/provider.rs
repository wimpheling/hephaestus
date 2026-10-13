//! Controlled exact owner response; no managed-root/kernel observation.
use super::controlled::vm::AutoExitProvider;
use async_trait::async_trait;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::broadcast;
use vm_trait::{
    StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec,
};
pub struct Provider {
    pub inner: AutoExitProvider,
    pub scope: VmProviderOwnerScope,
    pub fail: Arc<AtomicBool>,
    pub log: Arc<Mutex<Vec<&'static str>>>,
}
#[async_trait]
impl VmProvider for Provider {
    fn name(&self) -> &'static str {
        "controlled-strict-placement"
    }
    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        Ok(self.scope.clone())
    }
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(Instance {
            inner: self.inner.provision(spec).await?,
            fail: self.fail.clone(),
        }))
    }
    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState("unscoped cleanup forbidden"))
    }
    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<(), VmError> {
        if scope != &self.scope || id.0.parse::<uuid::Uuid>().is_err() {
            return Err(VmError::InvalidState("wrong controlled scope or identity"));
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(VmError::InvalidState("controlled physical uncertainty"));
        }
        self.log.lock().unwrap().push("scoped-confirmed");
        Ok(())
    }
}
struct Instance {
    inner: Arc<dyn VmInstance>,
    fail: Arc<AtomicBool>,
}
#[async_trait]
impl VmInstance for Instance {
    fn id(&self) -> &VmId {
        self.inner.id()
    }
    async fn start(&self) -> Result<(), VmError> {
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
        if self.fail.load(Ordering::SeqCst) {
            return Err(VmError::InvalidState("controlled destruction uncertain"));
        }
        self.inner.destroy().await
    }
}
