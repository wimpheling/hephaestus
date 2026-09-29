use super::*;
use crate::{GatewayEdgeError, GatewayServiceLaunch, GatewayServiceLaunchRequest};
use async_trait::async_trait;
use std::{
    future,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::broadcast;
use uuid::Uuid;
use vm_trait::{BoxedPrivateServiceConnection, StopMode, VmError, VmEvent, VmExit, VmSpec};

struct TestVm {
    id: VmId,
    destroys: AtomicUsize,
    destroy_fails: AtomicBool,
    destroy_pending: AtomicBool,
    events: broadcast::Sender<VmEvent>,
}

impl TestVm {
    fn new(id: VmId) -> Arc<Self> {
        let (events, _) = broadcast::channel(2);
        Arc::new(Self {
            id,
            destroys: AtomicUsize::new(0),
            destroy_fails: AtomicBool::new(false),
            destroy_pending: AtomicBool::new(false),
            events,
        })
    }
}

#[async_trait]
impl VmInstance for TestVm {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        Ok(())
    }

    async fn stop(&self, _: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        future::pending().await
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        Err(VmError::Unavailable {
            resource: String::from("test connection"),
            reason: String::from("not supported"),
        })
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        self.destroys.fetch_add(1, Ordering::Relaxed);
        if self.destroy_pending.load(Ordering::Relaxed) {
            future::pending().await
        } else if self.destroy_fails.load(Ordering::Relaxed) {
            Err(VmError::Destroyed)
        } else {
            Ok(())
        }
    }
}

struct TestProvider {
    calls: AtomicUsize,
    fails: AtomicBool,
    last_id: Mutex<Option<VmId>>,
}

#[async_trait]
impl VmProvider for TestProvider {
    fn name(&self) -> &'static str {
        "cleanup-test"
    }

    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Err(VmError::Unavailable {
            resource: String::from("test provision"),
            reason: String::from("not supported"),
        })
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.last_id.lock().expect("orphan id lock") = Some(id.clone());
        if self.fails.load(Ordering::Relaxed) {
            Err(VmError::Destroyed)
        } else {
            Ok(())
        }
    }
}

struct TestResolver {
    cleanups: AtomicUsize,
    cleanup_fails: AtomicBool,
    cleanup_identity: Mutex<Option<GatewayServiceIdentity>>,
}

#[async_trait]
impl GatewayServiceLaunchResolver for TestResolver {
    async fn resolve_service_launch(
        &self,
        _: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }

    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        *self.cleanup_identity.lock().expect("cleanup identity lock") = Some(identity);
        if self.cleanup_fails.load(Ordering::Relaxed) {
            Err(GatewayEdgeError::Unavailable)
        } else {
            Ok(())
        }
    }
}

fn identity() -> GatewayServiceIdentity {
    GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    }
}

fn provider() -> TestProvider {
    TestProvider {
        calls: AtomicUsize::new(0),
        fails: AtomicBool::new(false),
        last_id: Mutex::new(None),
    }
}

fn resolver() -> TestResolver {
    TestResolver {
        cleanups: AtomicUsize::new(0),
        cleanup_fails: AtomicBool::new(false),
        cleanup_identity: Mutex::new(None),
    }
}

#[tokio::test]
async fn retained_destroy_retry_preserves_same_handle() {
    let identity = identity();
    let vm = TestVm::new(VmId(format!("gateway-service-{}", identity.instance_id)));
    vm.destroy_fails.store(true, Ordering::Relaxed);
    let provider = provider();
    let resolver = resolver();
    let mut cleanup = GatewayServiceCleanup::new(
        identity,
        Some(Arc::clone(&vm) as Arc<dyn VmInstance>),
        Duration::from_secs(1),
    )
    .expect("cleanup");
    assert_eq!(
        cleanup.attempt(&provider, &resolver).await,
        Err(GatewayServiceCleanupError::VmTeardownIncomplete)
    );
    assert!(Arc::ptr_eq(
        &cleanup.retained_vm().expect("retained VM"),
        &(Arc::clone(&vm) as Arc<dyn VmInstance>)
    ));
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
    vm.destroy_fails.store(false, Ordering::Relaxed);
    cleanup.attempt(&provider, &resolver).await.expect("retry");
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 2);
    assert_eq!(
        *resolver
            .cleanup_identity
            .lock()
            .expect("cleanup identity lock"),
        Some(identity)
    );
    assert!(cleanup.vm_teardown_confirmed());
    assert!(cleanup.materializer_cleanup_confirmed());
}

#[tokio::test]
async fn orphan_failure_retains_materialization_and_destroy_success_skips_retry() {
    let identity = identity();
    let provider = provider();
    provider.fails.store(true, Ordering::Relaxed);
    let resolver = resolver();
    let mut cleanup =
        GatewayServiceCleanup::new(identity, None, Duration::from_secs(1)).expect("cleanup");
    assert_eq!(
        cleanup.attempt(&provider, &resolver).await,
        Err(GatewayServiceCleanupError::VmTeardownIncomplete)
    );
    assert!(!cleanup.vm_teardown_confirmed());
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
    provider.fails.store(false, Ordering::Relaxed);
    cleanup.attempt(&provider, &resolver).await.expect("retry");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        *provider.last_id.lock().expect("orphan id lock"),
        Some(VmId(format!("gateway-service-{}", identity.instance_id)))
    );
    assert_eq!(
        *resolver
            .cleanup_identity
            .lock()
            .expect("cleanup identity lock"),
        Some(identity)
    );
}

#[tokio::test]
async fn materializer_failure_does_not_redestroy_vm() {
    let identity = identity();
    let vm = TestVm::new(VmId(format!("gateway-service-{}", identity.instance_id)));
    let provider = provider();
    let resolver = resolver();
    resolver.cleanup_fails.store(true, Ordering::Relaxed);
    let mut cleanup = GatewayServiceCleanup::new(
        identity,
        Some(Arc::clone(&vm) as Arc<dyn VmInstance>),
        Duration::from_secs(1),
    )
    .expect("cleanup");
    assert_eq!(
        cleanup.attempt(&provider, &resolver).await,
        Err(GatewayServiceCleanupError::MaterializerCleanupIncomplete)
    );
    assert!(cleanup.vm_teardown_confirmed());
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    resolver.cleanup_fails.store(false, Ordering::Relaxed);
    cleanup.attempt(&provider, &resolver).await.expect("retry");
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 2);
    assert_eq!(
        *resolver
            .cleanup_identity
            .lock()
            .expect("cleanup identity lock"),
        Some(identity)
    );
}

#[tokio::test]
async fn confirmed_vm_teardown_retries_only_materializer_cleanup() {
    let identity = identity();
    let provider = provider();
    let resolver = resolver();
    let mut cleanup =
        GatewayServiceCleanup::from_progress(identity, None, true, false, Duration::from_secs(1))
            .expect("partial progress");
    cleanup.attempt(&provider, &resolver).await.expect("retry");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(cleanup.vm_teardown_confirmed());
    assert!(cleanup.materializer_cleanup_confirmed());
}

#[tokio::test]
async fn invalid_identity_vm_and_pending_destroy_fail_closed() {
    let invalid = GatewayServiceIdentity {
        instance_id: Uuid::nil(),
        ..identity()
    };
    assert!(matches!(
        GatewayServiceCleanup::new(invalid, None, Duration::from_secs(1)),
        Err(GatewayServiceCleanupError::InvalidInput)
    ));
    let identity = identity();
    let wrong_vm = TestVm::new(VmId(String::from("wrong")));
    assert!(matches!(
        GatewayServiceCleanup::new(
            identity,
            Some(Arc::clone(&wrong_vm) as Arc<dyn VmInstance>),
            Duration::from_secs(1),
        ),
        Err(GatewayServiceCleanupError::InvalidInput)
    ));
    assert!(matches!(
        GatewayServiceCleanup::new(identity, None, Duration::ZERO),
        Err(GatewayServiceCleanupError::InvalidInput)
    ));
    let vm = TestVm::new(VmId(format!("gateway-service-{}", identity.instance_id)));
    vm.destroy_pending.store(true, Ordering::Relaxed);
    let provider = provider();
    let resolver = resolver();
    let mut cleanup = GatewayServiceCleanup::new(
        identity,
        Some(Arc::clone(&vm) as Arc<dyn VmInstance>),
        Duration::from_millis(10),
    )
    .expect("cleanup");
    assert_eq!(
        cleanup.attempt(&provider, &resolver).await,
        Err(GatewayServiceCleanupError::VmTeardownIncomplete)
    );
    assert!(!cleanup.vm_teardown_confirmed());
    assert!(cleanup.retained_vm().is_some());
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
}
