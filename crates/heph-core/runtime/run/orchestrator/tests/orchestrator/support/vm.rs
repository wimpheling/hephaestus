use async_trait::async_trait;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;
use vm_trait::{StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec};

use super::helpers::lock;

pub struct AutoExitProvider {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
    pub spec: StdMutex<Option<VmSpec>>,
}

pub struct RevokeOnProvisionProvider {
    pub inner: AutoExitProvider,
    pub revoked: Arc<AtomicBool>,
}

#[async_trait]
impl VmProvider for RevokeOnProvisionProvider {
    fn name(&self) -> &'static str {
        "revoke-on-provision"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let instance = self.inner.provision(spec).await?;
        self.revoked.store(true, Ordering::SeqCst);
        Ok(instance)
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        self.inner.cleanup_orphan(id).await
    }
}

impl AutoExitProvider {
    pub const fn new(log: Arc<StdMutex<Vec<&'static str>>>) -> Self {
        Self {
            log,
            spec: StdMutex::new(None),
        }
    }

    pub fn spec(&self) -> Option<VmSpec> {
        lock(&self.spec).clone()
    }
}

#[async_trait]
impl VmProvider for AutoExitProvider {
    fn name(&self) -> &'static str {
        "auto-exit"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        lock(&self.log).push("provision");
        let runtime_authority = spec
            .runtime_authority
            .as_ref()
            .map(|authority| (authority.session_id(), authority.generation()));
        *lock(&self.spec) = Some(spec.clone());
        Ok(Arc::new(AutoExitInstance::new(
            spec.id,
            Arc::clone(&self.log),
            runtime_authority,
        )))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        lock(&self.log).push("orphan-cleanup");
        Ok(())
    }
}

pub struct AutoExitInstance {
    pub id: VmId,
    pub events: broadcast::Sender<VmEvent>,
    pub exit: watch::Sender<Option<VmExit>>,
    pub log: Arc<StdMutex<Vec<&'static str>>>,
    pub runtime_authority: Option<(Uuid, u64)>,
}

pub struct HangingProvider {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl VmProvider for HangingProvider {
    fn name(&self) -> &'static str {
        "hanging"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(HangingInstance {
            id: spec.id,
            log: Arc::clone(&self.log),
            events: broadcast::channel(8).0,
        }))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

pub struct HangingInstance {
    pub id: VmId,
    pub log: Arc<StdMutex<Vec<&'static str>>>,
    pub events: broadcast::Sender<VmEvent>,
}

#[async_trait]
impl VmInstance for HangingInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        lock(&self.log).push("stop");
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        std::future::pending().await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        lock(&self.log).push("destroy");
        Ok(())
    }
}

impl AutoExitInstance {
    pub fn new(
        id: VmId,
        log: Arc<StdMutex<Vec<&'static str>>>,
        runtime_authority: Option<(Uuid, u64)>,
    ) -> Self {
        let (events, _) = broadcast::channel(8);
        let (exit, _) = watch::channel(None);
        Self {
            id,
            events,
            exit,
            log,
            runtime_authority,
        }
    }
}

#[async_trait]
impl VmInstance for AutoExitInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        lock(&self.log).push("start");
        let _started = self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        });
        if let Some((session_id, generation)) = self.runtime_authority {
            let _acknowledgement = self.events.send(VmEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            });
        }
        let _ready = self.events.send(VmEvent::Ready);
        let _finalize = self.events.send(VmEvent::FinalizeResult {
            message: String::from("test result"),
        });
        let exit = VmExit {
            code: Some(0),
            signal: None,
        };
        self.exit.send_replace(Some(exit.clone()));
        let _exited = self.events.send(VmEvent::Exited(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current = receiver.borrow_and_update().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("test exit channel closed"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        lock(&self.log).push("destroy");
        Ok(())
    }
}
