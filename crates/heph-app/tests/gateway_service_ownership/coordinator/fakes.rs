use super::*;

pub async fn wait_for_status(
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

pub async fn coordinator_worker_pool() -> PgPool {
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

pub struct FakeResolver {
    launch: GatewayServiceLaunch,
    pub vm: Arc<FakeVm>,
    pub cleanups: AtomicUsize,
    pub cleanup_after_destroy: AtomicBool,
}

impl FakeResolver {
    pub fn new(identity: GatewayServiceIdentity, vm: Arc<FakeVm>) -> Self {
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
    ) -> Result<GatewayServiceLaunch, gateway_domain::GatewayEdgeError> {
        if request.identity != self.launch.identity {
            return Err(gateway_domain::GatewayEdgeError::Contract(
                "fake launch identity mismatch",
            ));
        }
        Ok(self.launch.clone())
    }

    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), gateway_domain::GatewayEdgeError> {
        if identity != self.launch.identity {
            return Err(gateway_domain::GatewayEdgeError::Contract(
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

pub struct FakeProvider {
    pub vm: Arc<FakeVm>,
}

impl FakeProvider {
    pub fn new(identity: GatewayServiceIdentity) -> Self {
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

pub struct FakeVm {
    id: VmId,
    events: broadcast::Sender<VmEvent>,
    readiness: Arc<Notify>,
    readiness_released: std::sync::atomic::AtomicBool,
    starts: AtomicUsize,
    pub destroys: AtomicUsize,
}

impl FakeVm {
    pub fn new(identity: GatewayServiceIdentity) -> Self {
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

    pub fn release_readiness(&self) {
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

pub fn fake_launch(identity: GatewayServiceIdentity) -> GatewayServiceLaunch {
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
            runtime_git_bridge: None,
            private_http_service: Some(PrivateHttpServiceSpec {
                loopback_port: 18_080,
                max_connections: 32,
                connect_timeout: Duration::from_secs(2),
            }),
            labels: BTreeMap::new(),
        },
    }
}
