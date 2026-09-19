//! Real `PostgreSQL` coordinator integration using a deterministic fake VM.

use super::{active_pointer, seed_fixture, seed_service_revision, test_pool};
use async_trait::async_trait;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayLimits, GatewayRequest, GatewayScheme, GatewayServiceCoordinator,
    GatewayServiceCoordinatorStatus, GatewayServiceIdentity, GatewayServiceInstanceKey,
    GatewayServiceLaunch, GatewayServiceLaunchRequest, GatewayServiceLaunchResolver,
    GatewayServiceLeasePolicy, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceRegistry, GatewayServiceStartupIntent, ServiceHttpPolicy, ServiceInstancePolicy,
    TrustedRequestMetadata,
};
use gateway_postgres::{PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets};
use http::{HeaderMap, Method, StatusCode};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex},
    sync::{Notify, broadcast},
    time::{Instant, timeout},
};
use uuid::Uuid;
use vm_trait::{
    BoxedPrivateServiceConnection, GuestCommand, NetworkMode, PrivateHttpServiceSpec,
    RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmResources,
    VmSpec,
};

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn coordinator_promotes_ready_service_and_cleans_real_ownership() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(candidate)
    .execute(&pool)
    .await
    .expect("desired candidate");

    let worker = coordinator_worker_pool().await;
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(worker.clone()));
    let targets = Arc::new(PostgresGatewayServiceTargets::new(worker));
    let owner = GatewayServiceOwner::new("coordinator-db-host", Uuid::new_v4()).expect("owner");
    let claim_started = Instant::now();
    let lease = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("claim desired candidate");
    let provider = Arc::new(FakeProvider::new(lease.identity));
    let resolver = Arc::new(FakeResolver::new(lease.identity, Arc::clone(&provider.vm)));
    let registry = GatewayServiceRegistry::new(2, 2).expect("registry");
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease.clone(),
        owner.clone(),
        claim_started + Duration::from_secs(30),
        claim_started + Duration::from_secs(10),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership,
        resolver.clone(),
        provider.clone(),
        targets,
        registry.clone(),
        "service.test",
        ServiceInstancePolicy::new(
            Duration::from_secs(5),
            Duration::from_millis(5),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        GatewayServiceLeasePolicy {
            lease_duration: Duration::from_secs(30),
            renewal_interval: Duration::from_secs(5),
        },
    )
    .expect("coordinator");
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Probing).await;
    provider.vm.release_readiness();
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Ready).await;

    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(candidate)
    );
    let state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(lease.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("ready state");
    assert_eq!(state, "ready");
    let key = GatewayServiceInstanceKey {
        identity: lease.identity,
        fencing_token: lease.fencing_token,
    };
    let response = registry
        .exchange(
            key,
            GatewayRequest {
                method: Method::GET,
                path_and_query: String::from("/ready"),
                headers: HeaderMap::new(),
                body: Vec::new().into(),
                trusted: TrustedRequestMetadata {
                    scheme: GatewayScheme::Http,
                    authority: String::from("service.test"),
                    client_address: "127.0.0.1".parse().expect("client address"),
                    request_id: Uuid::new_v4(),
                },
            },
            ServiceHttpPolicy::from_gateway_limits(GatewayLimits {
                max_request_body_bytes: 1024,
                max_response_body_bytes: 1024,
                max_request_headers: 16,
                max_response_headers: 16,
                max_path_and_query_bytes: 256,
                execution_timeout: Duration::from_secs(1),
            }),
        )
        .await
        .expect("registry exchange");
    assert_eq!(response.status, StatusCode::OK);

    drop(registry);
    control.cancel();
    assert!(
        timeout(Duration::from_secs(5), task)
            .await
            .expect("coordinator shutdown")
            .expect("coordinator join")
            .is_ok()
    );
    let final_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(lease.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("cleaned state");
    assert_eq!(final_state, "cleaned");
    let remaining_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_runtime_authority_sessions
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(candidate)
    .fetch_one(&pool)
    .await
    .expect("session-free service startup");
    assert_eq!(remaining_sessions, 0);
    assert_eq!(provider.vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(resolver.cleanup_after_destroy.load(Ordering::Relaxed));
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn coordinator_restores_old_active_without_overwriting_new_desired_revision() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let old_active = seed_service_revision(&pool, fixture.gateway).await;
    let desired = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = $3
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(old_active)
    .bind(desired)
    .execute(&pool)
    .await
    .expect("active and desired revisions");

    let worker = coordinator_worker_pool().await;
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(worker.clone()));
    let targets = Arc::new(PostgresGatewayServiceTargets::new(worker));
    let owner =
        GatewayServiceOwner::new("coordinator-restore-host", Uuid::new_v4()).expect("owner");
    let claim_started = Instant::now();
    let lease = ownership
        .claim_new(fixture.gateway, old_active, &owner, Duration::from_secs(30))
        .await
        .expect("claim old active revision");
    let provider = Arc::new(FakeProvider::new(lease.identity));
    let resolver = Arc::new(FakeResolver::new(lease.identity, Arc::clone(&provider.vm)));
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease.clone(),
        owner,
        claim_started + Duration::from_secs(30),
        claim_started + Duration::from_secs(10),
        GatewayServiceStartupIntent::RestoreActive,
        ownership,
        resolver.clone(),
        provider.clone(),
        targets,
        GatewayServiceRegistry::new(2, 2).expect("registry"),
        "service.test",
        ServiceInstancePolicy::new(
            Duration::from_secs(5),
            Duration::from_millis(5),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ),
        GatewayServiceLeasePolicy {
            lease_duration: Duration::from_secs(30),
            renewal_interval: Duration::from_secs(5),
        },
    )
    .expect("coordinator");
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Probing).await;
    provider.vm.release_readiness();
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Ready).await;
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(old_active)
    );
    let desired_pointer: Option<Uuid> =
        sqlx::query_scalar("SELECT desired_service_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("desired pointer");
    assert_eq!(desired_pointer, Some(desired));

    control.cancel();
    assert!(
        timeout(Duration::from_secs(5), task)
            .await
            .expect("coordinator shutdown")
            .expect("coordinator join")
            .is_ok()
    );
    let final_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(lease.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("cleaned restore state");
    assert_eq!(final_state, "cleaned");
    let remaining_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_runtime_authority_sessions
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(old_active)
    .fetch_one(&pool)
    .await
    .expect("session-free restored startup");
    assert_eq!(remaining_sessions, 0);
    assert!(resolver.cleanup_after_destroy.load(Ordering::Relaxed));
}

async fn wait_for_status(
    status: &mut tokio::sync::watch::Receiver<GatewayServiceCoordinatorStatus>,
    expected: GatewayServiceCoordinatorStatus,
) {
    timeout(Duration::from_secs(10), async {
        loop {
            if *status.borrow() == expected {
                return;
            }
            status.changed().await.expect("coordinator status");
        }
    })
    .await
    .expect("coordinator reached expected status");
}

async fn coordinator_worker_pool() -> PgPool {
    let database_url =
        std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("coordinator test database URL");
    PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-coordinator-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect coordinator worker PostgreSQL pool")
}

struct FakeResolver {
    launch: GatewayServiceLaunch,
    vm: Arc<FakeVm>,
    cleanups: AtomicUsize,
    cleanup_after_destroy: AtomicBool,
}

impl FakeResolver {
    fn new(identity: GatewayServiceIdentity, vm: Arc<FakeVm>) -> Self {
        Self {
            launch: fake_launch(identity),
            vm,
            cleanups: AtomicUsize::new(0),
            cleanup_after_destroy: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for FakeResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, gateway_edge::GatewayEdgeError> {
        if request.identity != self.launch.identity {
            return Err(gateway_edge::GatewayEdgeError::Contract(
                "fake launch identity mismatch",
            ));
        }
        Ok(self.launch.clone())
    }

    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), gateway_edge::GatewayEdgeError> {
        if identity != self.launch.identity {
            return Err(gateway_edge::GatewayEdgeError::Contract(
                "fake cleanup identity mismatch",
            ));
        }
        self.cleanup_after_destroy.store(
            self.vm.destroys.load(Ordering::Acquire) > 0,
            Ordering::Release,
        );
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct FakeProvider {
    vm: Arc<FakeVm>,
}

impl FakeProvider {
    fn new(identity: GatewayServiceIdentity) -> Self {
        Self {
            vm: Arc::new(FakeVm::new(identity)),
        }
    }
}

#[async_trait]
impl VmProvider for FakeProvider {
    fn name(&self) -> &'static str {
        "gateway-coordinator-test"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        if spec.id != self.vm.id {
            return Err(VmError::InvalidState("fake VM identity mismatch"));
        }
        Ok(self.vm.clone())
    }

    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct FakeVm {
    id: VmId,
    events: broadcast::Sender<VmEvent>,
    readiness: Arc<Notify>,
    readiness_released: std::sync::atomic::AtomicBool,
    starts: AtomicUsize,
    destroys: AtomicUsize,
}

impl FakeVm {
    fn new(identity: GatewayServiceIdentity) -> Self {
        let (events, _) = broadcast::channel(4);
        Self {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            events,
            readiness: Arc::new(Notify::new()),
            readiness_released: std::sync::atomic::AtomicBool::new(false),
            starts: AtomicUsize::new(0),
            destroys: AtomicUsize::new(0),
        }
    }

    fn release_readiness(&self) {
        self.readiness_released.store(true, Ordering::Release);
        self.readiness.notify_one();
    }
}

#[async_trait]
impl VmInstance for FakeVm {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        self.starts.fetch_add(1, Ordering::Relaxed);
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
        if !self.readiness_released.load(Ordering::Acquire) {
            self.readiness.notified().await;
        }
        let (host, mut guest) = duplex(8192);
        tokio::spawn(async move {
            let mut request = [0_u8; 2048];
            let _ = guest.read(&mut request).await;
            let _ = guest
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .await;
        });
        Ok(Box::new(host))
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        self.destroys.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn fake_launch(identity: GatewayServiceIdentity) -> GatewayServiceLaunch {
    let service = GatewayServiceConfig::new(
        18_080,
        ServiceProbePath::parse("/ready").expect("readiness path"),
        ServiceProbePath::parse("/health").expect("health path"),
    )
    .expect("service config");
    GatewayServiceLaunch {
        identity,
        service,
        spec: VmSpec {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/tmp/gateway-coordinator-test"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 64,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/service"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            private_http_service: Some(PrivateHttpServiceSpec {
                loopback_port: 18_080,
                max_connections: 32,
                connect_timeout: Duration::from_secs(2),
            }),
            labels: BTreeMap::new(),
        },
    }
}
