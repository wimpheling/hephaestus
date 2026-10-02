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
    #[cfg(test)]
    pub(super) fn ownership_test_inner(&self) -> &ProviderInner {
        &self.inner
    }
    /// Constructs an explicitly owned provider on a fresh or already managed root.
    ///
    /// Existing unclassified runtime roots cannot be adopted automatically.
    /// The configured host comes from trusted operator configuration.
    ///
    /// # Errors
    ///
    /// Rejects invalid backend configuration or mismatching durable ownership.
    pub fn new_owned(config: LibkrunConfig, host_id: &str) -> Result<Self, VmError> {
        let provider = Self::construct(config, Arc::new(ProcessWorkerSpawner))?;
        let owner = super::ownership::ProviderOwner::initialize(&provider.inner.config, host_id)?;
        provider
            .inner
            .owner
            .set(owner)
            .map_err(|_| VmError::InvalidState("VM owner is already configured"))?;
        Ok(provider)
    }
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
        super::ownership::reject_unowned(&config)?;
        Self::construct(config, worker_spawner)
    }

    fn construct(
        config: LibkrunConfig,
        worker_spawner: Arc<dyn WorkerSpawner>,
    ) -> Result<Self, VmError> {
        validate_config(&config)?;
        Ok(Self {
            inner: Arc::new(ProviderInner {
                config: Arc::new(config),
                ids: Mutex::new(HashSet::new()),
                worker_spawner,
                owner: std::sync::OnceLock::new(),
                failed_cleanup: Mutex::new(HashMap::new()),
            }),
        })
    }
}

#[async_trait]
impl VmProvider for LibkrunProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    fn owner_scope(&self) -> Result<vm_trait::VmProviderOwnerScope, VmError> {
        let owner = self.inner.owner.get().ok_or_else(|| VmError::Unsupported {
            feature: "persistent VM provider ownership".into(),
            provider: PROVIDER_NAME.into(),
        })?;
        let _guard = owner.validate(&self.inner.config)?;
        owner.scope()
    }

    async fn cleanup_orphan_scoped(
        &self,
        scope: &vm_trait::VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<(), VmError> {
        use std::os::fd::AsRawFd;
        validate_id(id)?;
        let owner = self.inner.owner.get().ok_or_else(|| VmError::Unsupported {
            feature: "scoped VM orphan cleanup".into(),
            provider: PROVIDER_NAME.into(),
        })?;
        if owner.scope()?.ne(scope) {
            return Err(VmError::InvalidState("VM cleanup owner scope differs"));
        }
        let guard = owner.validate(&self.inner.config)?;
        if let Some(instance) = {
            let failed_cleanup = self.inner.failed_cleanup.lock().await;
            let retained = failed_cleanup.get(id).cloned();
            drop(failed_cleanup);
            retained
        } {
            // Retry worker termination/reap explicitly; a previous failed
            // destroy may already have changed its in-memory lifecycle state.
            instance.force_cleanup(true).await?;
            drop(instance);
            self.inner.failed_cleanup.lock().await.remove(id);
            owner.validate_guard(&self.inner.config, &guard)?;
            return Ok(());
        }
        let ids = self.inner.ids.lock().await;
        if ids.contains(id) {
            return Err(VmError::InvalidState(
                "cannot clean an orphan while a live instance handle is registered",
            ));
        }
        // Pin both roots through cleanup, so path replacement cannot redirect IO
        // into another owner. Revalidate configured paths before confirming it.
        let mut pinned = self.inner.config.as_ref().clone();
        pinned.runtime_root = format!("/proc/self/fd/{}", guard.runtime.as_raw_fd()).into();
        pinned.cgroup_root = format!("/proc/self/fd/{}", guard.cgroup.as_raw_fd()).into();
        Cgroup::existing(&pinned, &id.0).cleanup()?;
        cleanup_runtime(&pinned.runtime_root.join(&id.0))?;
        owner.validate_guard(&self.inner.config, &guard)?;
        drop(ids);
        Ok(())
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        if self.inner.owner.get().is_none() {
            super::ownership::reject_unowned(&self.inner.config)?;
        }
        let owner_guard = self
            .inner
            .owner
            .get()
            .map(|owner| owner.validate(&self.inner.config))
            .transpose()?;
        let prepared = prepare_spec(&self.inner.config, &spec)?;
        if self
            .inner
            .failed_cleanup
            .lock()
            .await
            .contains_key(&spec.id)
        {
            return Err(VmError::AlreadyExists(spec.id));
        }
        {
            let mut ids = self.inner.ids.lock().await;
            if !ids.insert(spec.id.clone()) {
                return Err(VmError::AlreadyExists(spec.id));
            }
        }

        match self
            .provision_inner(spec.id.clone(), prepared, owner_guard.as_ref())
            .await
        {
            Ok(instance) => {
                if let Some((owner, guard)) = self.inner.owner.get().zip(owner_guard.as_ref()) {
                    if let Err(error) = owner.validate_guard(&self.inner.config, guard) {
                        // The exact worker and pinned allocation roots survive
                        // until confirmed destruction. A cleanup error remains
                        // uncertain and must not produce a scoped success.
                        if let Err(cleanup_error) = instance.destroy().await {
                            self.inner
                                .failed_cleanup
                                .lock()
                                .await
                                .insert(spec.id.clone(), instance);
                            return Err(cleanup_error);
                        }
                        return Err(error);
                    }
                }
                Ok(instance)
            }
            Err(error) => {
                self.inner.ids.lock().await.remove(&spec.id);
                Err(error)
            }
        }
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        if let Some(owner) = self.inner.owner.get() {
            return self.cleanup_orphan_scoped(&owner.scope()?, id).await;
        }
        super::ownership::reject_unowned(&self.inner.config)?;
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
        owner_guard: Option<&super::ownership::OwnerGuard>,
    ) -> Result<Arc<LibkrunInstance>, VmError> {
        let cleanup_roots = owner_guard
            .map(super::ownership::PinnedCleanupRoots::from_guard)
            .transpose()?;
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
                cleanup_roots,
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
