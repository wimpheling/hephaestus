use async_trait::async_trait;
use run_domain::Run;
use run_orchestrator::{RunAuthorizationError, RunLaunchAuthorizer, VmSpecFactory};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmProviderOwnerScope, VmSpec};
use volume_domain::RunVolumeSelections;

use super::super::super::support::{HangingProvider, TestSpecFactory};
use super::super::Fixture;

#[derive(Default)]
pub struct Authorizer {
    pub denied: AtomicBool,
    pub hung: AtomicBool,
    pub checks: AtomicUsize,
    pub entered: tokio::sync::Notify,
}

#[async_trait]
impl RunLaunchAuthorizer for Authorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        panic!("plural execution must not use the legacy authorization shortcut");
    }
    async fn authorize_with_volumes(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.hung.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        if self.denied.load(Ordering::SeqCst) {
            return Err(RunAuthorizationError::redacted("source/caller withdrawn"));
        }
        Ok(())
    }
}

pub struct Factory;
#[async_trait]
impl VmSpecFactory for Factory {
    async fn build(&self, _run: &Run) -> Result<VmSpec, VmError> {
        panic!("plural execution must not use scalar factory");
    }
    async fn build_with_volumes(
        &self,
        run: &Run,
        _selections: &RunVolumeSelections,
    ) -> Result<VmSpec, VmError> {
        TestSpecFactory.build(run).await
    }
}

pub struct HangingOwnedProvider {
    pub inner: HangingProvider,
    pub owner: Arc<super::super::provider::ScopedProvider>,
}
#[async_trait]
impl VmProvider for HangingOwnedProvider {
    fn name(&self) -> &'static str {
        "hanging-owned"
    }
    fn owner_scope(&self) -> Result<VmProviderOwnerScope, VmError> {
        self.owner.owner_scope()
    }
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.inner.provision(spec).await
    }
    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Err(VmError::InvalidState(
            "unscoped cleanup is unsupported by owned test provider",
        ))
    }
    async fn cleanup_orphan_scoped(
        &self,
        scope: &VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<(), VmError> {
        self.owner.cleanup_orphan_scoped(scope, id).await
    }
}

pub fn hanging_provider(fixture: &Fixture) -> Arc<dyn VmProvider> {
    Arc::new(HangingOwnedProvider {
        inner: HangingProvider {
            log: fixture.log.clone(),
        },
        owner: fixture.provider.clone(),
    })
}
