//! Bounded retryable cleanup for one persistent gateway service identity.

use std::{sync::Arc, time::Duration};

use tokio::time;
use vm_trait::{VmId, VmInstance, VmProvider};

use crate::{
    GatewayServiceIdentity, GatewayServiceLaunchResolver, service_instance::MAX_SHUTDOWN_TIMEOUT,
};

/// Redacted errors from one bounded physical cleanup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceCleanupError {
    /// The identity, VM ID, or timeout was invalid.
    #[error("invalid gateway service cleanup input")]
    InvalidInput,
    /// VM teardown did not complete within the bounded attempt.
    #[error("gateway service VM teardown is incomplete")]
    VmTeardownIncomplete,
    /// Materializer cleanup did not complete after VM teardown.
    #[error("gateway service materializer cleanup is incomplete")]
    MaterializerCleanupIncomplete,
}

/// Retryable physical cleanup state for one exact service identity.
pub struct GatewayServiceCleanup {
    identity: GatewayServiceIdentity,
    vm: Option<Arc<dyn VmInstance>>,
    vm_teardown_confirmed: bool,
    materializer_cleanup_confirmed: bool,
    timeout: Duration,
}

impl GatewayServiceCleanup {
    /// Creates cleanup state for a retained VM or a deterministic orphan ID.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupError::InvalidInput`] for nil identity
    /// fields, an invalid deterministic VM ID, or an out-of-bounds timeout.
    pub fn new(
        identity: GatewayServiceIdentity,
        vm: Option<Arc<dyn VmInstance>>,
        timeout: Duration,
    ) -> Result<Self, GatewayServiceCleanupError> {
        if identity.instance_id.is_nil()
            || identity.gateway_id.is_nil()
            || identity.revision_id.is_nil()
            || timeout.is_zero()
            || timeout > MAX_SHUTDOWN_TIMEOUT
        {
            return Err(GatewayServiceCleanupError::InvalidInput);
        }
        if vm
            .as_ref()
            .is_some_and(|vm| vm.id() != &Self::vm_id(identity))
        {
            return Err(GatewayServiceCleanupError::InvalidInput);
        }
        Ok(Self {
            identity,
            vm,
            vm_teardown_confirmed: false,
            materializer_cleanup_confirmed: false,
            timeout,
        })
    }

    /// Transfers caller-proven physical cleanup progress into retry state.
    ///
    /// The caller must have already confirmed both provider teardown and exact
    /// materializer cleanup.  This constructor never infers completion from a
    /// missing VM handle.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupError::InvalidInput`] when the identity
    /// or timeout violates the cleanup bounds.
    pub fn from_confirmed_physical(
        identity: GatewayServiceIdentity,
        timeout: Duration,
    ) -> Result<Self, GatewayServiceCleanupError> {
        let mut cleanup = Self::new(identity, None, timeout)?;
        cleanup.vm_teardown_confirmed = true;
        cleanup.materializer_cleanup_confirmed = true;
        Ok(cleanup)
    }

    /// Returns the exact service identity owned by this cleanup state.
    #[must_use]
    pub const fn identity(&self) -> GatewayServiceIdentity {
        self.identity
    }

    /// Returns whether provider teardown has been confirmed.
    #[must_use]
    pub const fn vm_teardown_confirmed(&self) -> bool {
        self.vm_teardown_confirmed
    }

    /// Returns whether materializer cleanup has been confirmed.
    #[must_use]
    pub const fn materializer_cleanup_confirmed(&self) -> bool {
        self.materializer_cleanup_confirmed
    }

    /// Returns the retained VM handle, when teardown still needs retrying.
    #[must_use]
    pub fn retained_vm(&self) -> Option<Arc<dyn VmInstance>> {
        self.vm.clone()
    }

    /// Attempts provider teardown and then exact materializer cleanup.
    ///
    /// The caller owns this state and may retry the same operation while
    /// retaining it alongside lease monitoring. No task is spawned here.
    ///
    /// # Errors
    ///
    /// Returns a redacted error while preserving the state needed for retry.
    pub async fn attempt(
        &mut self,
        provider: &dyn VmProvider,
        resolver: &dyn GatewayServiceLaunchResolver,
    ) -> Result<(), GatewayServiceCleanupError> {
        if !self.vm_teardown_confirmed {
            let teardown = if let Some(vm) = self.vm.as_ref() {
                time::timeout(self.timeout, vm.destroy())
                    .await
                    .is_ok_and(|result| result.is_ok())
            } else {
                time::timeout(
                    self.timeout,
                    provider.cleanup_orphan(&Self::vm_id(self.identity)),
                )
                .await
                .is_ok_and(|result| result.is_ok())
            };
            if !teardown {
                return Err(GatewayServiceCleanupError::VmTeardownIncomplete);
            }
            self.vm = None;
            self.vm_teardown_confirmed = true;
        }
        if !self.materializer_cleanup_confirmed {
            time::timeout(self.timeout, resolver.cleanup_service_launch(self.identity))
                .await
                .is_ok_and(|result| result.is_ok())
                .then_some(())
                .ok_or(GatewayServiceCleanupError::MaterializerCleanupIncomplete)?;
            self.materializer_cleanup_confirmed = true;
        }
        Ok(())
    }

    fn vm_id(identity: GatewayServiceIdentity) -> VmId {
        VmId(format!("gateway-service-{}", identity.instance_id))
    }
}

#[cfg(test)]
mod tests {
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
}
