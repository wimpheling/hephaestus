//! Controlled ports supply wait status and scoped absence; no VM is provisioned.
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use vm_trait::{
    StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec,
};

pub struct ControlledGuest {
    id: VmId,
    owner: VmProviderOwnerScope,
    calls: Mutex<Vec<&'static str>>,
    events: broadcast::Sender<VmEvent>,
}
impl ControlledGuest {
    pub fn new(f: &super::fixture::Fixture) -> Self {
        Self {
            id: f.birth.vm.clone(),
            owner: f.birth.config.provider_scope().clone(),
            calls: Mutex::new(Vec::new()),
            events: broadcast::channel(1).0,
        }
    }
    pub fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}
#[async_trait]
impl VmInstance for ControlledGuest {
    fn id(&self) -> &VmId {
        &self.id
    }
    async fn start(&self) -> Result<(), VmError> {
        let mut calls = self.calls.lock().unwrap();
        assert!(calls.is_empty());
        calls.push("start");
        drop(calls);
        Ok(())
    }
    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        panic!("positive controlled guest must exit through wait")
    }
    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut calls = self.calls.lock().unwrap();
        assert_eq!(*calls, ["start"]);
        calls.push("wait");
        drop(calls);
        // This supplied exit comes from the instance port, never persisted Run/outbox/receipt data.
        Ok(VmExit {
            code: Some(0),
            signal: None,
        })
    }
    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }
    async fn destroy(&self) -> Result<(), VmError> {
        let mut calls = self.calls.lock().unwrap();
        assert_eq!(*calls, ["start", "wait"]);
        calls.push("destroy");
        drop(calls);
        Ok(())
    }
}
#[async_trait]
impl VmProvider for ControlledGuest {
    fn name(&self) -> &'static str {
        "controlled-terminal-port"
    }
    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        Ok(self.owner.clone())
    }
    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<(), VmError> {
        assert_eq!(*scope, self.owner_scope()?);
        assert_eq!(id, self.id());
        let mut calls = self.calls.lock().unwrap();
        assert_eq!(*calls, ["start", "wait", "destroy"]);
        calls.push("scoped_confirmation");
        drop(calls);
        assert_eq!(*scope, self.owner_scope()?);
        Ok(())
    }
    async fn provision(&self, _spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        panic!("this fixture claims no native VM provisioning")
    }
    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        panic!("unscoped cleanup cannot produce this receipt")
    }
}
