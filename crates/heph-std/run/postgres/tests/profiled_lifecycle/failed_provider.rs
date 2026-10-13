//! A controlled destroy denial; no host resource observation is fabricated.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::broadcast;
use vm_trait::{StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec};

use super::controlled::vm::AutoExitProvider;

pub struct FailedDestroyProvider {
    pub inner: AutoExitProvider,
}

#[async_trait]
impl VmProvider for FailedDestroyProvider {
    fn name(&self) -> &'static str {
        "controlled-destroy-denial"
    }
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(FailedDestroyInstance {
            inner: self.inner.provision(spec).await?,
        }))
    }
    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState("controlled cleanup denied"))
    }
}

struct FailedDestroyInstance {
    inner: Arc<dyn VmInstance>,
}

#[async_trait]
impl VmInstance for FailedDestroyInstance {
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
        Err(VmError::InvalidState("controlled destroy denied"))
    }
}
