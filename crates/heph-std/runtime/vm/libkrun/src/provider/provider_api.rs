use super::common::{
    Arc, AtomicU64, Cgroup, Duration, EVENT_CAPACITY, HashMap, HashSet, LibkrunConfig,
    LibkrunInstance, Lifecycle, Mutex, OwnedResources, PROVIDER_NAME, PreparedSpec,
    ProcessWorkerSpawner, ProviderInner, Semaphore, ServiceBroker, VmError, VmId, VmInstance,
    VmProvider, VmSpec, WorkerSpawner, async_trait, broadcast, fs, info, prepare_spec,
    validate_config, validate_id, watch,
};
use super::helpers::{cleanup_runtime, create_runtime_dir, unavailable_error};

/// Fedora/Linux VM provider backed by a dedicated libkrun worker per VM.
#[derive(Clone)]
pub struct LibkrunProvider {
    inner: Arc<ProviderInner>,
}

impl LibkrunProvider {
    /// Validates host configuration and constructs a provider.
    ///
    /// This checks paths, the delegated cgroup, effective service identity,
    /// executable availability, and `/dev/kvm` access. libkrun/libkrunfw are
    /// loaded by each dedicated worker during provisioning.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidSpec`] for invalid configuration and
    /// [`VmError::Unavailable`] when a required host resource is unavailable.
    pub fn new(config: LibkrunConfig) -> Result<Self, VmError> {
        Self::new_with_spawner(config, Arc::new(ProcessWorkerSpawner))
    }

    pub(super) fn new_with_spawner(
        config: LibkrunConfig,
        worker_spawner: Arc<dyn WorkerSpawner>,
    ) -> Result<Self, VmError> {
        validate_config(&config)?;
        Ok(Self {
            inner: Arc::new(ProviderInner {
                config: Arc::new(config),
                ids: Mutex::new(HashSet::new()),
                worker_spawner,
            }),
        })
    }
}

#[async_trait]
impl VmProvider for LibkrunProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        let prepared = prepare_spec(&self.inner.config, &spec)?;
        {
            let mut ids = self.inner.ids.lock().await;
            if !ids.insert(spec.id.clone()) {
                return Err(VmError::AlreadyExists(spec.id));
            }
        }

        match self.provision_inner(spec.id.clone(), prepared).await {
            Ok(instance) => Ok(instance),
            Err(error) => {
                self.inner.ids.lock().await.remove(&spec.id);
                Err(error)
            }
        }
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        validate_id(id)?;
        if self.inner.ids.lock().await.contains(id) {
            return Err(VmError::InvalidState(
                "cannot clean an orphan while a live instance handle is registered",
            ));
        }

        Cgroup::existing(&self.inner.config, &id.0).cleanup()?;
        cleanup_runtime(&self.inner.config.runtime_root.join(&id.0))
    }
}

impl LibkrunProvider {
    async fn provision_inner(
        &self,
        id: VmId,
        spec: PreparedSpec,
    ) -> Result<Arc<dyn VmInstance>, VmError> {
        let runtime_dir = create_runtime_dir(&self.inner.config.runtime_root, &id.0)?;
        let cgroup = match Cgroup::create(&self.inner.config, &id.0) {
            Ok(cgroup) => cgroup,
            Err(error) => {
                let _cleanup_result = fs::remove_dir_all(&runtime_dir);
                return Err(error);
            }
        };

        info!(
            vm_id = %id.0,
            vcpus = spec.vcpus,
            memory_mib = spec.memory_mib,
            cpu_quota_micros = ?self.inner.config.limits.cpu_quota_micros,
            pids_max = self.inner.config.limits.pids_max,
            writable_disk_max_bytes = self.inner.config.limits.writable_disk_max_bytes,
            wall_clock_seconds = self.inner.config.limits.wall_clock_timeout.as_secs(),
            "provisioning VM resources"
        );
        let private_http_enabled = spec
            .labels
            .get(crate::protocol::GATEWAY_HANDLER_CONTRACT_LABEL)
            .is_some_and(|value| value == crate::protocol::GATEWAY_HANDLER_CONTRACT_V1);
        let private_service_timeout = spec
            .private_http_service
            .as_ref()
            .map(|service| Duration::from_millis(service.connect_timeout_ms));
        let private_service_dispatch = spec
            .private_http_service
            .as_ref()
            .map(|service| Arc::new(Semaphore::new(service.max_connections as usize)));
        let service_broker = match spec.private_http_service.as_ref() {
            Some(service) => match ServiceBroker::bind(
                &runtime_dir,
                service.max_connections,
                private_service_timeout.expect("service timeout is present"),
            ) {
                Ok(broker) => Some(Arc::new(broker)),
                Err(error) => {
                    let _cgroup_result = cgroup.cleanup();
                    let _runtime_result = fs::remove_dir_all(&runtime_dir);
                    return Err(unavailable_error(
                        "private service broker",
                        error.to_string(),
                    ));
                }
            },
            None => None,
        };
        let worker = match self
            .inner
            .worker_spawner
            .spawn(Arc::clone(&self.inner.config), spec, &runtime_dir, &cgroup)
            .await
        {
            Ok(worker) => worker,
            Err(error) => {
                if let Some(broker) = service_broker.as_ref() {
                    broker.shutdown().await;
                }
                let _cgroup_result = cgroup.cleanup();
                let _runtime_result = fs::remove_dir_all(&runtime_dir);
                return Err(error);
            }
        };

        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (terminal, _) = watch::channel(None);
        let (ready, _) = watch::channel(false);
        let (start_result, _) = watch::channel(None);
        let instance = Arc::new(LibkrunInstance {
            id,
            config: Arc::clone(&self.inner.config),
            worker,
            state: Mutex::new(Lifecycle::Provisioned),
            terminal,
            terminal_guard: Mutex::new(()),
            ready,
            start_result,
            events,
            resources: Mutex::new(Some(OwnedResources {
                runtime_dir,
                cgroup,
                service_broker,
            })),
            provider_ids: Arc::clone(&self.inner),
            private_http_enabled,
            private_service_timeout,
            private_service_dispatch,
            private_http_waiters: Mutex::new(HashMap::new()),
            next_private_http_request: AtomicU64::new(1),
        });
        instance.spawn_event_forwarder();
        instance.spawn_process_monitor();
        Ok(instance)
    }
}
