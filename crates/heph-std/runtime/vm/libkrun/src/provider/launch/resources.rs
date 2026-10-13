//! Existing original allocation, instance installation and resource cleanup.
use super::PendingLaunch;
use crate::provider::{
    common::{
        Arc, AtomicBool, AtomicU64, Cgroup, Duration, EVENT_CAPACITY, HashMap, LibkrunInstance,
        Lifecycle, Mutex, OwnedResources, PreparedSpec, ProviderInner, Semaphore, VmError,
        WorkerBackend, broadcast, watch,
    },
    helpers::{provider_error, unavailable_error},
    ownership::OwnerGuard,
};
use std::os::fd::AsRawFd;

impl PendingLaunch {
    pub(super) async fn launch(
        &self,
        provider: &ProviderInner,
        spec: PreparedSpec,
        guard: Option<&OwnerGuard>,
    ) -> Result<Arc<dyn WorkerBackend>, VmError> {
        if let Some((owner, guard)) = provider.owner.get().zip(guard) {
            owner.require_containment()?;
            owner.validate_guard(&provider.config, guard)?;
        }
        let mut allocation = provider.config.as_ref().clone();
        if let Some(guard) = guard {
            allocation.runtime_root =
                format!("/proc/self/fd/{}/.", guard.runtime.as_raw_fd()).into();
            allocation.cgroup_root = format!("/proc/self/fd/{}/.", guard.cgroup.as_raw_fd()).into();
        }
        let mut resources = self.resources.lock().await;
        let runtime = provider.config.runtime_root.join(&self.id.0);
        let runtime_child = crate::directory::ChildDirectory::create(
            &allocation.runtime_root,
            &self.id.0,
            rustix::fs::Mode::from_bits_truncate(0o700),
            |child| resources.runtime = Some(child),
        )
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => VmError::AlreadyExists(self.id.clone()),
            _ => provider_error("runtime-create", error),
        })?;
        // chmod targets the acquired inode, retaining exclusive allocation if
        // permissions fail rather than following a redirected child name.
        runtime_child
            .file()
            .and_then(|file| {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o700))
            })
            .map_err(|error| provider_error("runtime-permissions", error))?;
        self.custody.retain_runtime(runtime_child)?;
        let placement = Cgroup::create_tracked(&allocation, &self.id.0, |cgroup| {
            resources.cgroup = Some(cgroup.clone());
        })?;
        if let Some((owner, guard)) = provider.owner.get().zip(guard) {
            super::containment::BirthRecord::publish(
                (&guard.runtime, &guard.cgroup),
                &owner.scope()?,
                &self.id,
                (
                    &*self.custody.runtime_file()?,
                    &*placement
                        .allocated_directory()?
                        .file()
                        .map_err(|error| provider_error("birth-cgroup-identity", error))?,
                ),
                |record| resources.birth_record = Some(record),
            )?;
        }
        if let Some(service) = spec.private_http_service.as_ref() {
            let duration = Duration::from_millis(service.connect_timeout_ms);
            resources.broker = Some(Arc::new(
                crate::service_transport::ServiceBroker::bind_pinned(
                    self.custody.runtime_file()?,
                    service.max_connections,
                    duration,
                )
                .map_err(|error| unavailable_error("private service broker", error.to_string()))?,
            ));
        }
        drop(resources);
        // Controls and transport use the actual allocated child descriptors,
        // retained by this owner before the spawner's first asynchronous wait.
        let backend = provider
            .worker_spawner
            .spawn(
                Arc::clone(&provider.config),
                spec,
                &runtime,
                &placement,
                &self.custody,
            )
            .await?;
        self.custody.record_backend(Arc::clone(&backend))?;
        if let Some((owner, guard)) = provider.owner.get().zip(guard) {
            owner.validate_guard(&provider.config, guard)?;
        }
        let resources = self.resources.lock().await;
        let runtime = resources
            .runtime
            .as_ref()
            .ok_or(VmError::InvalidState("launch runtime allocation missing"))?;
        if !runtime
            .validate_named()
            .map_err(|error| provider_error("runtime-identity", error))?
        {
            return Err(unavailable_error(
                "runtime identity",
                "allocated runtime was removed",
            ));
        }
        resources
            .cgroup
            .as_ref()
            .ok_or(VmError::InvalidState("launch cgroup allocation missing"))?
            .validate_allocation()?;
        drop(resources);
        Ok(backend)
    }

    pub(super) async fn install_instance(
        &self,
        provider: &Arc<ProviderInner>,
        worker: Arc<dyn WorkerBackend>,
        settings: InstanceSettings,
    ) -> Result<Arc<LibkrunInstance>, VmError> {
        let pid = self.custody.process()?.and_then(|process| process.pid);
        let mut resources = self.resources.lock().await;
        let runtime_child = resources
            .runtime
            .as_ref()
            .ok_or(VmError::InvalidState("launch runtime allocation missing"))?
            .clone();
        let runtime_dir = provider.config.runtime_root.join(&self.id.0);
        let cgroup = resources
            .cgroup
            .take()
            .ok_or(VmError::InvalidState("launch cgroup allocation missing"))?;
        resources.runtime = None;
        let owned = OwnedResources {
            runtime_dir,
            runtime_child: Some(runtime_child),
            birth_record: resources.birth_record.take(),
            cgroup,
            service_broker: resources.broker.take(),
            cleanup_roots: resources.roots.take(),
        };
        drop(resources);
        let instance = Arc::new_cyclic(|this| LibkrunInstance {
            this: this.clone(),
            id: self.id.clone(),
            config: Arc::clone(&provider.config),
            worker,
            state: Mutex::new(Lifecycle::Provisioned),
            operation: Mutex::new(()),
            destroy_requested: watch::channel(false).0,
            stop_requested: watch::channel(false).0,
            cleanup_complete: AtomicBool::new(false),
            terminal: watch::channel(None).0,
            terminal_guard: Mutex::new(()),
            ready: watch::channel(false).0,
            start_result: watch::channel(None).0,
            events: broadcast::channel(EVENT_CAPACITY).0,
            resources: Mutex::new(Some(owned)),
            provider_ids: Arc::clone(&provider.lifetime),
            registry: Arc::downgrade(&provider.registry),
            birth: self.birth.load(super::common::Ordering::Acquire),
            private_http_enabled: settings.http_enabled,
            private_service_timeout: settings.service_timeout,
            private_service_dispatch: settings.dispatch,
            private_http_waiters: Mutex::new(HashMap::new()),
            next_private_http_request: AtomicU64::new(1),
        });
        provider
            .registry
            .install(instance.birth, Arc::clone(&instance), pid);
        instance.spawn_event_forwarder();
        instance.spawn_process_monitor();
        Ok(instance)
    }

    pub(super) async fn cleanup(&self, provider: &ProviderInner) -> Result<(), VmError> {
        self.custody
            .terminate(provider.config.startup_timeout)
            .await?;
        let mut resources = self.resources.lock().await;
        if let Some(broker) = resources.broker.as_ref() {
            broker.shutdown().await;
        }
        if let Some(cgroup) = resources.cgroup.as_ref() {
            cgroup.cleanup()?;
        }
        if let Some(runtime) = resources.runtime.as_ref() {
            runtime
                .remove(true)
                .map_err(|error| provider_error("runtime-cleanup", error))?;
        }
        if let Some(record) = resources.birth_record.as_ref() {
            record.remove_original()?;
        }
        resources.birth_record = None;
        resources.runtime = None;
        resources.cgroup = None;
        resources.broker = None;
        drop(resources);
        Ok(())
    }
}

pub(in crate::provider) struct InstanceSettings {
    http_enabled: bool,
    service_timeout: Option<Duration>,
    dispatch: Option<Arc<Semaphore>>,
}

impl InstanceSettings {
    pub(in crate::provider) fn from_spec(spec: &PreparedSpec) -> Self {
        Self {
            http_enabled: spec
                .labels
                .get(crate::protocol::GATEWAY_HANDLER_CONTRACT_LABEL)
                .is_some_and(|value| value == crate::protocol::GATEWAY_HANDLER_CONTRACT_V1),
            service_timeout: spec
                .private_http_service
                .as_ref()
                .map(|service| Duration::from_millis(service.connect_timeout_ms)),
            dispatch: private_service_dispatch(spec),
        }
    }
}

// Preserve the reviewed pure extraction from the strict-lint successor.
fn private_service_dispatch(spec: &PreparedSpec) -> Option<Arc<Semaphore>> {
    spec.private_http_service
        .as_ref()
        .map(|service| Arc::new(Semaphore::new(service.max_connections as usize)))
}
