use crate::{
    GatewayServiceFailure, GatewayServiceFailureStore, GatewayServiceIdentity,
    GatewayServiceInstanceLease, GatewayServiceLaunchResolver, GatewayServiceOwner,
    GatewayServiceTargetStore,
};
use async_trait::async_trait;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use uuid::Uuid;
use vm_trait::{VmError, VmId, VmInstance, VmProvider, VmSpec};

pub(super) struct TestTargets {
    pub(super) instances: Mutex<Vec<GatewayServiceInstanceLease>>,
    pub(super) inventory: Mutex<Vec<GatewayServiceInstanceLease>>,
    pub(super) list_calls: AtomicUsize,
    pub(super) hide_after: Option<usize>,
    pub(super) cleaned: Option<Arc<Mutex<HashSet<Uuid>>>>,
}

#[async_trait]
impl GatewayServiceTargetStore for TestTargets {
    async fn list_service_targets(
        &self,
        _: crate::GatewayServiceTargetPage,
    ) -> Result<crate::GatewayServiceTargetPageResult, crate::GatewayEdgeError> {
        Ok(crate::GatewayServiceTargetPageResult {
            targets: Vec::new(),
            next_after: None,
        })
    }

    async fn get_service_target(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<Option<crate::GatewayServiceOwnedTarget>, crate::GatewayEdgeError> {
        Ok(None)
    }

    async fn count_accepted_service_invocations(
        &self,
        _: Uuid,
        _: Uuid,
    ) -> Result<u64, crate::GatewayEdgeError> {
        Ok(0)
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        _: crate::GatewayServiceInstanceKey,
    ) -> Result<u64, crate::GatewayEdgeError> {
        Ok(0)
    }

    async fn get_service_instance(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, crate::GatewayEdgeError> {
        Ok(self
            .instances
            .lock()
            .expect("instances")
            .iter()
            .find(|lease| lease.identity == identity)
            .cloned())
    }

    async fn list_service_instances(
        &self,
        _: crate::GatewayServiceInstancePage,
    ) -> Result<crate::GatewayServiceInstancePageResult, crate::GatewayEdgeError> {
        let call = self.list_calls.fetch_add(1, Ordering::Relaxed) + 1;
        let instances = if self.hide_after.is_some_and(|limit| call > limit) {
            Vec::new()
        } else {
            self.inventory
                .lock()
                .expect("inventory")
                .iter()
                .filter(|lease| {
                    self.cleaned.as_ref().is_none_or(|cleaned| {
                        !cleaned
                            .lock()
                            .expect("cleaned")
                            .contains(&lease.identity.instance_id)
                    })
                })
                .cloned()
                .collect()
        };
        Ok(crate::GatewayServiceInstancePageResult {
            instances,
            next_after: None,
        })
    }
}

pub(super) struct TestFailureStore;

#[async_trait]
impl GatewayServiceFailureStore for TestFailureStore {
    async fn record_failure(
        &self,
        _: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        _: GatewayServiceFailure,
    ) -> Result<(), crate::GatewayServiceFailureStoreError> {
        Ok(())
    }
}

pub(super) struct TestResolver;

#[async_trait]
impl GatewayServiceLaunchResolver for TestResolver {
    async fn resolve_service_launch(
        &self,
        _: crate::GatewayServiceLaunchRequest,
    ) -> Result<crate::GatewayServiceLaunch, crate::GatewayEdgeError> {
        Err(crate::GatewayEdgeError::Unavailable)
    }

    async fn cleanup_service_launch(
        &self,
        _: GatewayServiceIdentity,
    ) -> Result<(), crate::GatewayEdgeError> {
        Ok(())
    }
}

pub(super) struct TestProvider {
    pub(super) physical: Option<Arc<Mutex<HashSet<String>>>>,
    pub(super) block_started: Option<Arc<Notify>>,
    pub(super) block_release: Option<Arc<Notify>>,
    pub(super) block_count: Option<Arc<AtomicUsize>>,
}

impl TestProvider {
    pub(super) fn standard() -> Self {
        Self {
            physical: None,
            block_started: None,
            block_release: None,
            block_count: None,
        }
    }
}

#[async_trait]
impl VmProvider for TestProvider {
    fn name(&self) -> &'static str {
        "boot-recovery-test"
    }

    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Err(VmError::Unavailable {
            resource: String::from("test provider"),
            reason: String::from("not used"),
        })
    }

    async fn cleanup_orphan(&self, vm_id: &VmId) -> Result<(), VmError> {
        if let Some(physical) = &self.physical {
            physical.lock().expect("physical").insert(vm_id.0.clone());
        }
        if let (Some(started), Some(release), Some(count)) =
            (&self.block_started, &self.block_release, &self.block_count)
        {
            if count.fetch_add(1, Ordering::Relaxed) + 1 == 2 {
                started.notify_waiters();
            }
            release.notified().await;
        }
        Ok(())
    }
}
