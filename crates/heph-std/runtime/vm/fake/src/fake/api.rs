use super::{
    EVENT_CAPACITY, FakeInstance, InstanceState, PrivateHttpResponder, ProviderInner, lock,
    validation::validate_spec,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmSpec};

/// A deterministic provider that simulates VMs without starting processes.
///
/// This provider is intended for core lifecycle and orchestration tests. It
/// validates provider-neutral specifications, allocates fake host ports, and
/// implements the same idempotency and exit-caching contract as real
/// providers.
#[derive(Clone)]
pub struct FakeProvider {
    inner: Arc<ProviderInner>,
}

impl FakeProvider {
    /// Creates an empty fake provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProviderInner::default()),
        }
    }

    /// Configures a deterministic private HTTP guest handler for instances
    /// provisioned by this provider.  It remains available with
    /// [`vm_trait::NetworkMode::Disabled`], proving this path does not require a guest
    /// listener or port forwarding.
    #[must_use]
    pub fn with_private_http_responder(self, responder: Arc<dyn PrivateHttpResponder>) -> Self {
        *lock(&self.inner.private_http_responder) = Some(responder);
        self
    }
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VmProvider for FakeProvider {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        validate_spec(&spec)?;

        {
            let mut ids = lock(&self.inner.ids);
            if !ids.insert(spec.id.clone()) {
                return Err(VmError::AlreadyExists(spec.id));
            }
        }

        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (terminal, _) = watch::channel(None);
        let instance = FakeInstance {
            id: spec.id.clone(),
            spec,
            provider: Arc::clone(&self.inner),
            state: Mutex::new(InstanceState::Provisioned),
            events,
            terminal,
        };

        Ok(Arc::new(instance))
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        if lock(&self.inner.ids).contains(id) {
            return Err(VmError::InvalidState(
                "cannot clean an orphan while a live instance handle is registered",
            ));
        }
        Ok(())
    }
}
