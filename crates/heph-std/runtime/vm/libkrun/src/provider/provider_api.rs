#[path = "managed_configuration.rs"]
mod managed_configuration;

#[path = "managed_owner.rs"]
mod managed_owner;
pub use managed_owner::ProspectiveOwnedVmBootstrap;

use super::common::{
    Arc, Cgroup, HashMap, HashSet, LibkrunConfig, Mutex, PROVIDER_NAME, PendingLaunch,
    ProcessWorkerSpawner, ProviderInner, ProviderLifetime, VmError, VmId, VmInstance, VmProvider,
    VmSpec, WorkerSpawner, async_trait, oneshot, prepare_spec, validate_config, validate_id,
};
use super::helpers::{cleanup_runtime, unavailable_error};

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
    #[cfg(test)]
    pub(super) fn launch_test_inner(&self) -> Arc<ProviderInner> {
        Arc::clone(&self.inner)
    }
    /// Closes provision and guest Start admission for this provider generation.
    ///
    /// Already admitted operations retain custody and may finish. Closing does
    /// not stop workers or prove physical drain, and has no reopen operation.
    ///
    /// # Errors
    ///
    /// Rejects unowned providers, mismatching scope, or invalid current ownership.
    pub async fn close_admission(
        &self,
        expected: &vm_trait::VmProviderOwnerScope,
    ) -> Result<(), VmError> {
        let owner = self.scoped_owner(expected)?;
        let guard = owner.validate(&self.inner.config)?;
        let ids = self.inner.ids.lock().await;
        owner.validate_guard(&self.inner.config, &guard)?;
        self.inner.registry.close();
        drop(ids);
        Ok(())
    }

    /// Returns diagnostics for every birth admitted in this supervisor generation.
    ///
    /// This is not cleanup authority or a native drain receipt. An empty new
    /// generation does not account for orphans from a prior supervisor process.
    ///
    /// # Errors
    ///
    /// Rejects unowned providers, mismatching scope, or invalid current ownership.
    pub async fn attempt_inventory(
        &self,
        expected: &vm_trait::VmProviderOwnerScope,
    ) -> Result<super::registry::VmAttemptInventory, VmError> {
        let owner = self.scoped_owner(expected)?;
        let guard = owner.validate(&self.inner.config)?;
        let inventory = self.inner.registry.inventory().await;
        owner.validate_guard(&self.inner.config, &guard)?;
        Ok(inventory)
    }

    /// Observes no residual VM births under this original configured owner.
    ///
    /// Call after qualified durable per-Run recovery and before command admission.
    /// This one-time read-only cooperative observation neither closes admission
    /// nor proves host-wide absence, database cleanup, or a physical drain receipt.
    ///
    /// # Errors
    /// Rejects unconfigured/old owners, emulated cgroups, live generation activity,
    /// residual or unknown allocations, and changed original roots or metadata.
    pub async fn assert_no_prior_births(
        &self,
        expected: &vm_trait::VmProviderOwnerScope,
    ) -> Result<(), VmError> {
        let owner = self.scoped_owner(expected)?;
        validate_config(&self.inner.config)?;
        let guard = owner.validate(&self.inner.config)?;
        let ids = self.inner.ids.lock().await;
        let result = self.inner.registry.observe_settled(|| {
            if !ids.is_empty() {
                return Err(VmError::InvalidState("VM identities remain admitted"));
            }
            owner.assert_no_prior_births(&self.inner.config, &guard)
        });
        drop(ids);
        result
    }

    /// Closes admission and cleans all births known to this live generation.
    ///
    /// The owning task continues through caller cancellation. Success requires
    /// confirmed cleanup of these exact births and completion of their launch
    /// and lifecycle tasks. It is not a global physical drain receipt: an empty
    /// reopened generation says nothing about prior-process orphans.
    ///
    /// # Errors
    ///
    /// Rejects unowned/wrong-scope providers or invalid ownership. Unconfirmed
    /// cleanup returns an error and retains the original custody for retry.
    pub async fn drain_generation(
        &self,
        expected: &vm_trait::VmProviderOwnerScope,
    ) -> Result<(), VmError> {
        let owner = self.scoped_owner(expected)?;
        let guard = owner.validate(&self.inner.config)?;
        let provider = Arc::clone(&self.inner);
        let (completed, outcome) = oneshot::channel();
        // Original provider/root custody is transferred before the first await.
        tokio::spawn(async move {
            let result = super::drain::run(provider, guard).await;
            let _reported = completed.send(result);
        });
        outcome.await.map_err(|_| {
            unavailable_error("generation drain", "owning drain task channel closed")
        })?
    }

    fn scoped_owner(
        &self,
        expected: &vm_trait::VmProviderOwnerScope,
    ) -> Result<&super::ownership::ProviderOwner, VmError> {
        let owner = self.inner.owner.get().ok_or_else(|| VmError::Unsupported {
            feature: "scoped VM admission and inventory".into(),
            provider: PROVIDER_NAME.into(),
        })?;
        if owner.scope()?.ne(expected) {
            return Err(VmError::InvalidState("VM admission owner scope differs"));
        }
        Ok(owner)
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
                lifetime: Arc::new(ProviderLifetime {
                    config: Arc::new(config),
                    ids: Mutex::new(HashSet::new()),
                    worker_spawner,
                    owner: std::sync::OnceLock::new(),
                }),
                registry: super::registry::Registry::new(),
                failed_cleanup: Mutex::new(HashMap::new()),
                pending_launches: std::sync::Mutex::new(HashMap::new()),
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
        validate_id(id)?;
        let owner = self.inner.owner.get().ok_or_else(|| VmError::Unsupported {
            feature: "scoped VM orphan cleanup".into(),
            provider: PROVIDER_NAME.into(),
        })?;
        if owner.scope()?.ne(scope) {
            return Err(VmError::InvalidState("VM cleanup owner scope differs"));
        }
        owner.require_containment()?;
        let guard = owner.validate(&self.inner.config)?;
        if let Some(pending) = self.pending_launch(id) {
            pending.retry_cleanup().await?;
            owner.validate_guard(&self.inner.config, &guard)?;
            return Ok(());
        }
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
        if !self.inner.config.enforce_cgroup_v2 {
            return Err(VmError::Unsupported {
                feature: "prior-process containment requires actual cgroup-v2 controls".into(),
                provider: PROVIDER_NAME.into(),
            });
        }
        let gate = super::containment::recover(&self.inner.config, &guard, scope, id)?;
        owner.validate_guard(&self.inner.config, &guard)?;
        drop(ids);
        drop(gate);
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
        let (entry, retries) =
            PendingLaunch::new(spec.id.clone(), &self.inner.config, owner_guard.as_ref())?;
        let (result, receiver) = oneshot::channel();
        {
            let mut ids = self.inner.ids.lock().await;
            let mut pending = self
                .inner
                .pending_launches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending.contains_key(&entry.id) {
                return Err(VmError::AlreadyExists(entry.id.clone()));
            }
            self.inner
                .registry
                .admit(&self.inner.lifetime, &mut ids, Arc::clone(&entry))?;
            pending.insert(entry.id.clone(), Arc::clone(&entry));
            // Reservation, discoverable custody and owning-task installation
            // have no await gap. Caller cancellation drops only its receiver.
            tokio::spawn(super::launch::run_launch(
                Arc::clone(&self.inner),
                Arc::clone(&entry),
                prepared,
                owner_guard,
                result,
                retries,
            ));
            drop(pending);
            drop(ids);
        }
        let super::launch::LaunchHandoff { instance, accepted } = receiver
            .await
            .map_err(|_| unavailable_error("worker launch", "launch result channel closed"))??;
        // Cancellation while acquiring this lock still drops the acknowledgement,
        // leaving the owner task responsible for the same actual instance.
        let ids = self.inner.ids.lock().await;
        let mut pending = self
            .inner
            .pending_launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !pending
            .get(&entry.id)
            .is_some_and(|current| Arc::ptr_eq(current, &entry))
        {
            return Err(VmError::InvalidState("launch handoff entry changed"));
        }
        if entry.drain_requested() {
            return Err(unavailable_error(
                "worker launch",
                "generation drain requested",
            ));
        }
        // Acknowledge before retiring only this entry; preserve the ID reservation.
        // No await separates acknowledgement, retirement and returning the instance.
        accepted
            .send(())
            .map_err(|()| unavailable_error("worker launch", "launch handoff channel closed"))?;
        pending.remove(&entry.id);
        entry
            .state
            .send_replace(super::launch::LaunchState::HandedOff);
        drop(entry);
        drop(pending);
        drop(ids);
        Ok(instance)
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        if let Some(owner) = self.inner.owner.get() {
            return self.cleanup_orphan_scoped(&owner.scope()?, id).await;
        }
        super::ownership::reject_unowned(&self.inner.config)?;
        validate_id(id)?;
        if let Some(pending) = self.pending_launch(id) {
            return pending.retry_cleanup().await;
        }
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
    fn pending_launch(&self, id: &VmId) -> Option<Arc<PendingLaunch>> {
        self.inner
            .pending_launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }
}
