use super::*;
use async_trait::async_trait;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Notify, broadcast};
use vm_trait::{
    GuestCommand, NetworkMode, PrivateHttpServiceSpec, RootFilesystem, StopMode, VmError, VmEvent,
    VmExit, VmResources, VmSpec,
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
        runtime_git_bridge: None,
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

#[path = "preparation_scenarios.rs"]
mod preparation_scenarios;
