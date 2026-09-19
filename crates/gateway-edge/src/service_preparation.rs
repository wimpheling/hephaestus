//! Parent-owned preparation of one persistent gateway service instance.
//!
//! The parent must insert [`ServicePreparation::run`] into its own task set
//! and join it. Dropping [`ServicePreparationHandle`] requests cancellation;
//! it does not abandon an in-flight resolver or provider future. Forced
//! process shutdown relies on the persisted ownership ledger.

use std::fmt;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use vm_trait::{VmId, VmInstance, VmProvider};

use crate::{
    GatewayServiceIdentity, GatewayServiceLaunch, GatewayServiceLaunchRequest,
    GatewayServiceLaunchResolver,
};

/// A stopped VM paired with the exact immutable launch it was prepared for.
pub struct PreparedGatewayService {
    /// Immutable service launch selected by the durable resolver.
    pub launch: GatewayServiceLaunch,
    /// Stopped VM owned by the parent supervisor.
    pub vm: Arc<dyn VmInstance>,
}

impl fmt::Debug for PreparedGatewayService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedGatewayService")
            .field("identity", &self.launch.identity)
            .field("vm_id", self.vm.id())
            .finish()
    }
}

/// Redacted reason for a preparation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePreparationFailureReason {
    /// The resolver could not produce the exact immutable launch.
    Resolution,
    /// The resolver returned an invalid identity or VM specification.
    InvalidLaunch,
    /// Provider provisioning returned an error.
    Provisioning,
    /// The parent requested cancellation.
    Cancelled,
    /// Provider or materializer cleanup could not be confirmed.
    CleanupIncomplete,
}

/// Preparation failure retaining every live ownership handle required for retry.
pub struct ServicePreparationFailure {
    /// Exact service identity owned by this preparation attempt.
    pub identity: GatewayServiceIdentity,
    /// Redacted failure classification.
    pub reason: ServicePreparationFailureReason,
    /// Returned VM retained when destroy did not complete.
    pub vm: Option<Arc<dyn VmInstance>>,
    /// Whether the exact materialization may still exist and must be retained.
    pub materialization_owned: bool,
}

impl fmt::Debug for ServicePreparationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServicePreparationFailure")
            .field("identity", &self.identity)
            .field("reason", &self.reason)
            .field("vm_present", &self.vm.is_some())
            .field("materialization_owned", &self.materialization_owned)
            .finish()
    }
}

/// Cancellation control for one parent-owned preparation attempt.
pub struct ServicePreparationHandle {
    cancellation: CancellationToken,
}

impl ServicePreparationHandle {
    /// Requests cancellation at the next safe phase boundary.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl Drop for ServicePreparationHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Parent-owned preparation future.
pub struct ServicePreparation {
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    provider: Arc<dyn VmProvider>,
    request: GatewayServiceLaunchRequest,
    cancellation: CancellationToken,
}

/// Creates one parent-owned preparation future and its cancellation handle.
///
/// The parent must retain the returned handle while the future runs, keep the
/// durable ownership heartbeat alive, and join [`ServicePreparation::run`].
/// It must not drop or abort the running future except during forced process
/// shutdown, where the persisted ownership ledger drives recovery.
#[must_use]
pub fn new_service_preparation(
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    provider: Arc<dyn VmProvider>,
    request: GatewayServiceLaunchRequest,
) -> (ServicePreparationHandle, ServicePreparation) {
    let cancellation = CancellationToken::new();
    let handle = ServicePreparationHandle {
        cancellation: cancellation.clone(),
    };
    let preparation = ServicePreparation {
        resolver,
        provider,
        request,
        cancellation,
    };
    (handle, preparation)
}

impl ServicePreparation {
    /// Resolves, materializes, and provisions one stopped service VM.
    ///
    /// The resolver and provider futures are always awaited to settlement. A
    /// cancellation observed after provisioning destroys the returned VM
    /// before removing its exact materialization.
    ///
    /// # Errors
    ///
    /// Returns ownership of a VM or materialization whenever cleanup is not
    /// confirmed, allowing the parent to retry in the same process.
    pub async fn run(self) -> Result<PreparedGatewayService, ServicePreparationFailure> {
        let identity = self.request.identity;
        if self.cancellation.is_cancelled() {
            return Err(failure(
                identity,
                ServicePreparationFailureReason::Cancelled,
                None,
                false,
            ));
        }

        // Deliberately await to completion: dropping this future can abandon a
        // resolver that has already prepared host-owned release state.
        let Ok(launch) = self.resolver.resolve_service_launch(self.request).await else {
            return cleanup_without_vm(
                &self.resolver,
                identity,
                ServicePreparationFailureReason::Resolution,
            )
            .await;
        };
        if !valid_launch(&launch, identity) {
            return cleanup_without_vm(
                &self.resolver,
                identity,
                ServicePreparationFailureReason::InvalidLaunch,
            )
            .await;
        }
        if self.cancellation.is_cancelled() {
            return cleanup_without_vm(
                &self.resolver,
                identity,
                ServicePreparationFailureReason::Cancelled,
            )
            .await;
        }

        // The provider future is likewise never selected away or dropped.
        let Ok(vm) = self.provider.provision(launch.spec.clone()).await else {
            return cleanup_after_provision_error(&self.resolver, &self.provider, identity).await;
        };
        if self.cancellation.is_cancelled() {
            return cleanup_after_vm(&self.resolver, identity, vm).await;
        }
        Ok(PreparedGatewayService { launch, vm })
    }
}

async fn cleanup_without_vm(
    resolver: &Arc<dyn GatewayServiceLaunchResolver>,
    identity: GatewayServiceIdentity,
    reason: ServicePreparationFailureReason,
) -> Result<PreparedGatewayService, ServicePreparationFailure> {
    match resolver.cleanup_service_launch(identity).await {
        Ok(()) => Err(failure(identity, reason, None, false)),
        Err(_) => Err(failure(
            identity,
            ServicePreparationFailureReason::CleanupIncomplete,
            None,
            true,
        )),
    }
}

async fn cleanup_after_provision_error(
    resolver: &Arc<dyn GatewayServiceLaunchResolver>,
    provider: &Arc<dyn VmProvider>,
    identity: GatewayServiceIdentity,
) -> Result<PreparedGatewayService, ServicePreparationFailure> {
    let id = VmId(format!("gateway-service-{}", identity.instance_id));
    if provider.cleanup_orphan(&id).await.is_err() {
        return Err(failure(
            identity,
            ServicePreparationFailureReason::CleanupIncomplete,
            None,
            true,
        ));
    }
    cleanup_without_vm(
        resolver,
        identity,
        ServicePreparationFailureReason::Provisioning,
    )
    .await
}

async fn cleanup_after_vm(
    resolver: &Arc<dyn GatewayServiceLaunchResolver>,
    identity: GatewayServiceIdentity,
    vm: Arc<dyn VmInstance>,
) -> Result<PreparedGatewayService, ServicePreparationFailure> {
    if vm.destroy().await.is_err() {
        return Err(failure(
            identity,
            ServicePreparationFailureReason::CleanupIncomplete,
            Some(vm),
            true,
        ));
    }
    cleanup_without_vm(
        resolver,
        identity,
        ServicePreparationFailureReason::Cancelled,
    )
    .await
}

fn valid_launch(launch: &GatewayServiceLaunch, identity: GatewayServiceIdentity) -> bool {
    let expected_id = format!("gateway-service-{}", identity.instance_id);
    launch.identity == identity
        && !identity.instance_id.is_nil()
        && !identity.gateway_id.is_nil()
        && !identity.revision_id.is_nil()
        && launch.spec.id == VmId(expected_id)
        && launch.service.validate().is_ok()
        && launch
            .spec
            .private_http_service
            .as_ref()
            .is_some_and(|service| service.loopback_port == launch.service.loopback_port)
}

fn failure(
    identity: GatewayServiceIdentity,
    reason: ServicePreparationFailureReason,
    vm: Option<Arc<dyn VmInstance>>,
    materialization_owned: bool,
) -> ServicePreparationFailure {
    ServicePreparationFailure {
        identity,
        reason,
        vm,
        materialization_owned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::{Notify, broadcast};
    use vm_trait::{
        GuestCommand, NetworkMode, PrivateHttpServiceSpec, RootFilesystem, StopMode, VmError,
        VmEvent, VmExit, VmResources, VmSpec,
    };

    struct TestResolver {
        launch: GatewayServiceLaunch,
        block: AtomicBool,
        fail: AtomicBool,
        cleanup_fail: AtomicBool,
        resolve_started: Notify,
        release_resolve: Notify,
        cleanup_calls: AtomicUsize,
        destroyed_before_cleanup: Arc<AtomicBool>,
        cleanup_observed_destroy: AtomicBool,
    }

    impl TestResolver {
        fn new(launch: GatewayServiceLaunch) -> Arc<Self> {
            Arc::new(Self {
                launch,
                block: AtomicBool::new(false),
                fail: AtomicBool::new(false),
                cleanup_fail: AtomicBool::new(false),
                resolve_started: Notify::new(),
                release_resolve: Notify::new(),
                cleanup_calls: AtomicUsize::new(0),
                destroyed_before_cleanup: Arc::new(AtomicBool::new(false)),
                cleanup_observed_destroy: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl GatewayServiceLaunchResolver for TestResolver {
        async fn resolve_service_launch(
            &self,
            _: GatewayServiceLaunchRequest,
        ) -> Result<GatewayServiceLaunch, crate::GatewayEdgeError> {
            self.resolve_started.notify_one();
            if self.block.load(Ordering::Relaxed) {
                self.release_resolve.notified().await;
            }
            if self.fail.load(Ordering::Relaxed) {
                Err(crate::GatewayEdgeError::Unavailable)
            } else {
                Ok(self.launch.clone())
            }
        }

        async fn cleanup_service_launch(
            &self,
            _: GatewayServiceIdentity,
        ) -> Result<(), crate::GatewayEdgeError> {
            self.cleanup_calls.fetch_add(1, Ordering::Relaxed);
            self.cleanup_observed_destroy.store(
                self.destroyed_before_cleanup.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            if self.cleanup_fail.load(Ordering::Relaxed) {
                Err(crate::GatewayEdgeError::Unavailable)
            } else {
                Ok(())
            }
        }
    }

    struct TestProvider {
        block: AtomicBool,
        fail: AtomicBool,
        cleanup_fail: AtomicBool,
        provision_started: Notify,
        release_provision: Notify,
        cleanup_calls: AtomicUsize,
        vm: Arc<TestVm>,
    }

    impl TestProvider {
        fn new(vm: Arc<TestVm>) -> Arc<Self> {
            Arc::new(Self {
                block: AtomicBool::new(false),
                fail: AtomicBool::new(false),
                cleanup_fail: AtomicBool::new(false),
                provision_started: Notify::new(),
                release_provision: Notify::new(),
                cleanup_calls: AtomicUsize::new(0),
                vm,
            })
        }
    }

    #[async_trait]
    impl VmProvider for TestProvider {
        fn name(&self) -> &'static str {
            "preparation-test"
        }

        async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
            self.provision_started.notify_one();
            if self.block.load(Ordering::Relaxed) {
                self.release_provision.notified().await;
            }
            if self.fail.load(Ordering::Relaxed) {
                Err(VmError::Unavailable {
                    resource: String::from("test provider"),
                    reason: String::from("deliberate failure"),
                })
            } else {
                Ok(self.vm.clone())
            }
        }

        async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
            self.cleanup_calls.fetch_add(1, Ordering::Relaxed);
            if self.cleanup_fail.load(Ordering::Relaxed) {
                Err(VmError::InvalidState("provider cleanup is uncertain"))
            } else {
                Ok(())
            }
        }
    }

    struct TestVm {
        id: VmId,
        start_calls: AtomicUsize,
        destroy_calls: AtomicUsize,
        destroy_fail: AtomicBool,
        destroyed: Arc<AtomicBool>,
        events: broadcast::Sender<VmEvent>,
    }

    impl TestVm {
        fn new(id: VmId, destroyed: Arc<AtomicBool>) -> Arc<Self> {
            let (events, _) = broadcast::channel(4);
            Arc::new(Self {
                id,
                start_calls: AtomicUsize::new(0),
                destroy_calls: AtomicUsize::new(0),
                destroy_fail: AtomicBool::new(false),
                destroyed,
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
            self.start_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn stop(&self, _: StopMode) -> Result<(), VmError> {
            Ok(())
        }

        async fn wait(&self) -> Result<VmExit, VmError> {
            Err(VmError::InvalidState("test VM has not exited"))
        }

        fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
            self.events.subscribe()
        }

        async fn destroy(&self) -> Result<(), VmError> {
            self.destroy_calls.fetch_add(1, Ordering::Relaxed);
            self.destroyed.store(true, Ordering::Relaxed);
            if self.destroy_fail.load(Ordering::Relaxed) {
                Err(VmError::InvalidState("test destroy failed"))
            } else {
                Ok(())
            }
        }
    }

    fn identity() -> GatewayServiceIdentity {
        GatewayServiceIdentity {
            instance_id: uuid::Uuid::from_u128(1),
            gateway_id: uuid::Uuid::from_u128(2),
            revision_id: uuid::Uuid::from_u128(3),
        }
    }

    fn launch(identity: GatewayServiceIdentity) -> GatewayServiceLaunch {
        let readiness_path = ServiceProbePath::parse("/ready").unwrap();
        let health_path = ServiceProbePath::parse("/health").unwrap();
        let service = GatewayServiceConfig::new(8080, readiness_path, health_path).unwrap();
        let spec = VmSpec {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            root: RootFilesystem::Directory {
                host_path: "/tmp/test-root".into(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/service"),
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            private_http_service: Some(PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 32,
                connect_timeout: std::time::Duration::from_secs(2),
            }),
            labels: std::collections::BTreeMap::new(),
        };
        GatewayServiceLaunch {
            identity,
            service,
            spec,
        }
    }

    #[tokio::test]
    async fn cancellation_during_resolution_settles_then_cleans() {
        let identity = identity();
        let resolver = TestResolver::new(launch(identity));
        resolver.block.store(true, Ordering::Relaxed);
        let provider = TestProvider::new(TestVm::new(
            VmId(format!("gateway-service-{}", identity.instance_id)),
            resolver.destroyed_before_cleanup.clone(),
        ));
        let (handle, preparation) = new_service_preparation(
            resolver.clone(),
            provider.clone(),
            GatewayServiceLaunchRequest { identity },
        );
        let task = tokio::spawn(preparation.run());
        resolver.resolve_started.notified().await;
        drop(handle);
        resolver.release_resolve.notify_one();
        let failure = task.await.unwrap().unwrap_err();
        assert_eq!(failure.reason, ServicePreparationFailureReason::Cancelled);
        assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 1);
        assert_eq!(provider.cleanup_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn cancellation_during_provision_destroys_late_vm_before_materialization() {
        let identity = identity();
        let resolver = TestResolver::new(launch(identity));
        let vm = TestVm::new(
            VmId(format!("gateway-service-{}", identity.instance_id)),
            resolver.destroyed_before_cleanup.clone(),
        );
        let provider = TestProvider::new(vm.clone());
        provider.block.store(true, Ordering::Relaxed);
        let (handle, preparation) = new_service_preparation(
            resolver.clone(),
            provider.clone(),
            GatewayServiceLaunchRequest { identity },
        );
        let task = tokio::spawn(preparation.run());
        resolver.resolve_started.notified().await;
        provider.provision_started.notified().await;
        handle.cancel();
        provider.release_provision.notify_one();
        let failure = task.await.unwrap().unwrap_err();
        assert_eq!(failure.reason, ServicePreparationFailureReason::Cancelled);
        assert!(failure.vm.is_none());
        assert_eq!(vm.destroy_calls.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 1);
        assert!(resolver.cleanup_observed_destroy.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn failed_destroy_retains_vm_and_materialization() {
        let identity = identity();
        let resolver = TestResolver::new(launch(identity));
        let vm = TestVm::new(
            VmId(format!("gateway-service-{}", identity.instance_id)),
            resolver.destroyed_before_cleanup.clone(),
        );
        vm.destroy_fail.store(true, Ordering::Relaxed);
        let provider = TestProvider::new(vm.clone());
        provider.block.store(true, Ordering::Relaxed);
        let (handle, preparation) = new_service_preparation(
            resolver.clone(),
            provider.clone(),
            GatewayServiceLaunchRequest { identity },
        );
        let task = tokio::spawn(preparation.run());
        provider.provision_started.notified().await;
        handle.cancel();
        provider.release_provision.notify_one();
        let failure = task.await.unwrap().unwrap_err();
        assert_eq!(
            failure.reason,
            ServicePreparationFailureReason::CleanupIncomplete
        );
        assert!(failure.vm.is_some());
        assert!(failure.materialization_owned);
        assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn provider_error_requires_orphan_confirmation_before_materialization_cleanup() {
        let identity = identity();
        let resolver = TestResolver::new(launch(identity));
        let provider = TestProvider::new(TestVm::new(
            VmId(format!("gateway-service-{}", identity.instance_id)),
            resolver.destroyed_before_cleanup.clone(),
        ));
        provider.fail.store(true, Ordering::Relaxed);
        provider.cleanup_fail.store(true, Ordering::Relaxed);
        let (_handle, preparation) = new_service_preparation(
            resolver.clone(),
            provider.clone(),
            GatewayServiceLaunchRequest { identity },
        );
        let failure = preparation.run().await.unwrap_err();
        assert_eq!(
            failure.reason,
            ServicePreparationFailureReason::CleanupIncomplete
        );
        assert!(failure.materialization_owned);
        assert_eq!(provider.cleanup_calls.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn normal_preparation_returns_stopped_vm() {
        let identity = identity();
        let resolver = TestResolver::new(launch(identity));
        let vm = TestVm::new(
            VmId(format!("gateway-service-{}", identity.instance_id)),
            resolver.destroyed_before_cleanup.clone(),
        );
        let provider = TestProvider::new(vm.clone());
        let (_handle, preparation) =
            new_service_preparation(resolver, provider, GatewayServiceLaunchRequest { identity });
        let prepared = preparation.run().await.unwrap();
        assert_eq!(prepared.vm.id(), &vm.id);
        assert_eq!(vm.start_calls.load(Ordering::Relaxed), 0);
        assert_eq!(vm.destroy_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn resolver_error_still_attempts_exact_materialization_cleanup() {
        let identity = identity();
        let resolver = TestResolver::new(launch(identity));
        resolver.fail.store(true, Ordering::Relaxed);
        let vm = TestVm::new(
            VmId(format!("gateway-service-{}", identity.instance_id)),
            resolver.destroyed_before_cleanup.clone(),
        );
        let provider = TestProvider::new(vm);
        let (_handle, preparation) = new_service_preparation(
            resolver.clone(),
            provider,
            GatewayServiceLaunchRequest { identity },
        );
        let failure = preparation.run().await.unwrap_err();
        assert_eq!(failure.reason, ServicePreparationFailureReason::Resolution);
        assert!(!failure.materialization_owned);
        assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 1);
    }
}
