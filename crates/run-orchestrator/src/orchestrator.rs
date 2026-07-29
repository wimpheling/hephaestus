use async_trait::async_trait;
use run_domain::{CancelRun, Run, RunState, StartRun};
use runtime_types::RunId;
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use vm_trait::{
    DiskFormat, StopMode, VmDisk, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec,
};
use volume_trait::{
    INSTANCE_STATE_DISK_ID, VolumeAttachment, VolumeError, VolumeLease, VolumeStore,
};
use workspace_domain::{DisabledWorkspaceManager, RunWorkspaceManager, WorkspaceError};

use crate::{RepositoryError, RunRepository, StoredVmEvent};

/// Prepared immutable host-owned files for one exact run.
#[derive(Debug, Clone, Default)]
pub struct PreparedRunRuntime {
    /// Read-only mounts added to the VM specification.
    pub mounts: Vec<vm_trait::VmMount>,
}

/// Lifecycle boundary for exact release artifacts and host-generated context.
#[async_trait]
pub trait RunRuntimeManager: Send + Sync + 'static {
    /// Materializes a fresh, non-reusable runtime tree for one run.
    async fn prepare(&self, run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError>;
    /// Destroys runtime files after provider cleanup has been confirmed.
    async fn destroy(&self, run_id: RunId) -> Result<(), RunRuntimeError>;
    /// Reconciles runtime trees that no live run may reuse.
    async fn recover(&self) -> Result<usize, RunRuntimeError>;
}

/// Live authorization boundary for every logical artifact acquisition and VM
/// start.
#[async_trait]
pub trait RunLaunchAuthorizer: Send + Sync + 'static {
    /// Rechecks the exact run, attachment/update, and release authority.
    async fn authorize(&self, run: &Run) -> Result<(), RunAuthorizationError>;
}

#[derive(Debug)]
struct DisabledRunLaunchAuthorizer;

#[async_trait]
impl RunLaunchAuthorizer for DisabledRunLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        Ok(())
    }
}

/// Redacted live launch-authorization failure.
#[derive(Debug, thiserror::Error)]
#[error("run launch authorization failed: {message}")]
pub struct RunAuthorizationError {
    message: String,
}

impl RunAuthorizationError {
    /// Creates a redacted authorization failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Prepared ephemeral secret authority for one exact run.
#[derive(Debug, Clone, Default)]
pub struct PreparedRunSecrets {
    /// Read-only secret and runtime-credential mounts added to the VM.
    pub mounts: Vec<vm_trait::VmMount>,
}

/// Lifecycle boundary for exact per-run raw and brokered secret authority.
#[async_trait]
pub trait RunSecretManager: Send + Sync + 'static {
    /// Resolves live authority and materializes only this run's secret files.
    async fn prepare(&self, run: &Run) -> Result<PreparedRunSecrets, RunSecretError>;
    /// Rechecks every prepared lease immediately before VM provisioning.
    async fn reauthorize(&self, run: &Run) -> Result<(), RunSecretError>;
    /// Destroys secret authority after guest destruction is confirmed.
    async fn destroy_after_guest(&self, run_id: RunId) -> Result<(), RunSecretError>;
    /// Reconciles orphan secret files without deleting live guest authority.
    async fn recover(&self) -> Result<usize, RunSecretError>;
}

/// Receives terminal runs only after guest resources and the state lease have
/// been destroyed.
#[async_trait]
pub trait RunCompletionObserver: Send + Sync + 'static {
    /// Applies an idempotent domain decision for one cleaned run.
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError>;
    /// Reconciles cleaned runs whose domain decision was interrupted.
    async fn recover(&self) -> Result<usize, RunCompletionError>;
}

#[derive(Debug)]
struct DisabledRunCompletionObserver;

#[async_trait]
impl RunCompletionObserver for DisabledRunCompletionObserver {
    async fn after_cleanup(&self, _run: &Run) -> Result<(), RunCompletionError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        Ok(0)
    }
}

/// Redacted post-cleanup domain transition failure.
#[derive(Debug, thiserror::Error)]
#[error("run completion observation failed: {message}")]
pub struct RunCompletionError {
    message: String,
}

impl RunCompletionError {
    /// Creates a redacted completion failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct DisabledRunSecretManager;

#[async_trait]
impl RunSecretManager for DisabledRunSecretManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        Ok(PreparedRunSecrets::default())
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunSecretError> {
        Ok(())
    }

    async fn destroy_after_guest(&self, _run_id: RunId) -> Result<(), RunSecretError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunSecretError> {
        Ok(0)
    }
}

/// Non-disclosing exact-secret lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("run secret operation failed: {message}")]
pub struct RunSecretError {
    message: String,
}

impl RunSecretError {
    /// Creates a redacted secret lifecycle failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct DisabledRunRuntimeManager;

#[async_trait]
impl RunRuntimeManager for DisabledRunRuntimeManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        Ok(PreparedRunRuntime::default())
    }

    async fn destroy(&self, _run_id: RunId) -> Result<(), RunRuntimeError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
}

/// Provider-neutral exact-runtime lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("run runtime operation failed: {message}")]
pub struct RunRuntimeError {
    message: String,
}

impl RunRuntimeError {
    /// Creates a redacted runtime lifecycle failure.
    #[must_use]
    pub fn redacted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Builds the non-volume portion of a VM specification for a run.
#[async_trait]
pub trait VmSpecFactory: Send + Sync + 'static {
    /// Creates a VM specification. The orchestrator replaces any disk named
    /// `agent-state` with the currently leased attachment.
    ///
    /// # Errors
    ///
    /// Returns an error when run configuration cannot produce a valid spec.
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError>;
}

/// Durable coordinator for run, volume, and VM lifecycles.
pub struct RunOrchestrator {
    repository: Arc<dyn RunRepository>,
    volumes: Arc<dyn VolumeStore>,
    provider: Arc<dyn VmProvider>,
    spec_factory: Arc<dyn VmSpecFactory>,
    workspaces: Arc<dyn RunWorkspaceManager>,
    runtimes: Arc<dyn RunRuntimeManager>,
    launch_authorizer: Arc<dyn RunLaunchAuthorizer>,
    secrets: Arc<dyn RunSecretManager>,
    completion: Arc<dyn RunCompletionObserver>,
    active: Mutex<HashMap<RunId, Arc<dyn VmInstance>>>,
    instance_state_capacity_bytes: u64,
    cancellation_timeout: Duration,
}

impl RunOrchestrator {
    /// Creates an orchestrator from its durable and provider-neutral
    /// boundaries.
    #[must_use]
    pub fn new(
        repository: Arc<dyn RunRepository>,
        volumes: Arc<dyn VolumeStore>,
        provider: Arc<dyn VmProvider>,
        spec_factory: Arc<dyn VmSpecFactory>,
        instance_state_capacity_bytes: u64,
    ) -> Self {
        Self {
            repository,
            volumes,
            provider,
            spec_factory,
            workspaces: Arc::new(DisabledWorkspaceManager),
            runtimes: Arc::new(DisabledRunRuntimeManager),
            launch_authorizer: Arc::new(DisabledRunLaunchAuthorizer),
            secrets: Arc::new(DisabledRunSecretManager),
            completion: Arc::new(DisabledRunCompletionObserver),
            active: Mutex::new(HashMap::new()),
            instance_state_capacity_bytes,
            cancellation_timeout: Duration::from_secs(10),
        }
    }

    /// Installs the trusted repository workspace and result lifecycle manager.
    #[must_use]
    pub fn with_workspace_manager(mut self, workspaces: Arc<dyn RunWorkspaceManager>) -> Self {
        self.workspaces = workspaces;
        self
    }

    /// Installs exact release-artifact and host-context materialization.
    #[must_use]
    pub fn with_runtime_manager(mut self, runtimes: Arc<dyn RunRuntimeManager>) -> Self {
        self.runtimes = runtimes;
        self
    }

    /// Installs the live authorization boundary used before artifact
    /// materialization and immediately before VM provisioning.
    #[must_use]
    pub fn with_launch_authorizer(
        mut self,
        launch_authorizer: Arc<dyn RunLaunchAuthorizer>,
    ) -> Self {
        self.launch_authorizer = launch_authorizer;
        self
    }

    /// Installs live secret dispatch and ephemeral mount materialization.
    #[must_use]
    pub fn with_secret_manager(mut self, secrets: Arc<dyn RunSecretManager>) -> Self {
        self.secrets = secrets;
        self
    }

    /// Installs idempotent post-cleanup domain result processing.
    #[must_use]
    pub fn with_completion_observer(mut self, completion: Arc<dyn RunCompletionObserver>) -> Self {
        self.completion = completion;
        self
    }

    /// Executes an idempotent start command through complete cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error when durable state, volume, or VM operations fail.
    /// Cleanup failures deliberately retain the volume lease for recovery.
    // Keeping the ordered lifecycle in one method makes lease cleanup ordering
    // directly auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn start_run(&self, command: &StartRun) -> Result<Run, OrchestratorError> {
        let created = self.repository.create_run(command).await?;
        if !created.created {
            match created.run.state {
                RunState::Queued | RunState::LeasingVolume => {}
                RunState::CleanedUp => {
                    self.completion.after_cleanup(&created.run).await?;
                    return Ok(created.run);
                }
                _ => return Err(OrchestratorError::RunInProgress(command.run_id)),
            }
        }
        if created.run.state == RunState::Queued {
            self.repository
                .transition(command.run_id, RunState::LeasingVolume, None, None)
                .await?;
        }

        let attachment = if command.requires_state {
            let volume = match self
                .volumes
                .resolve_instance_state(command.instance_id, self.instance_state_capacity_bytes)
                .await
            {
                Ok(volume) => volume,
                Err(error) => {
                    return self
                        .fail_without_lease(command.run_id, &error.to_string())
                        .await;
                }
            };
            match self.volumes.acquire(volume.id, command.run_id).await {
                Ok(attachment) => Some(attachment),
                Err(error) => {
                    return self
                        .fail_without_lease(command.run_id, &error.to_string())
                        .await;
                }
            }
        } else {
            None
        };
        let vm_id = VmId(command.run_id.to_string());
        let volume_id = attachment.as_ref().map(|value| value.volume.id);
        let lease_id = attachment.as_ref().map(|value| value.lease.id);
        let run = self
            .repository
            .bind_resources(command.run_id, volume_id, lease_id, &vm_id.0)
            .await?;
        if run.cancel_requested_at.is_some() {
            return self
                .cancel_before_vm(run, attachment.as_ref().map(|value| &value.lease))
                .await;
        }

        self.repository
            .transition(command.run_id, RunState::Provisioning, None, None)
            .await?;
        if let Err(error) = self.launch_authorizer.authorize(&run).await {
            return self
                .fail_with_resources(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await;
        }
        let workspace = match self.workspaces.prepare(&run).await {
            Ok(workspace) => workspace,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await;
            }
        };
        let workspace_enabled = workspace.id.is_some();
        let runtime = match self.runtimes.prepare(&run).await {
            Ok(runtime) => runtime,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await;
            }
        };
        let mut mounts = workspace.mounts;
        mounts.extend(runtime.mounts);
        let secrets = match self.secrets.prepare(&run).await {
            Ok(secrets) => secrets,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await;
            }
        };
        mounts.extend(secrets.mounts);
        let spec = match self.build_spec(&run, attachment.as_ref(), mounts).await {
            Ok(spec) => spec,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await;
            }
        };
        let execution_timeout = spec
            .labels
            .get("hephaestus.wall-clock-timeout-seconds")
            .map(|seconds| {
                seconds
                    .parse::<u64>()
                    .ok()
                    .filter(|seconds| *seconds > 0)
                    .map(Duration::from_secs)
                    .ok_or_else(|| VmError::InvalidSpec {
                        field: String::from("labels.hephaestus.wall-clock-timeout-seconds"),
                        reason: String::from("must be a positive integer"),
                    })
            })
            .transpose()?;
        if let Err(error) = self.launch_authorizer.authorize(&run).await {
            return self
                .fail_with_resources(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await;
        }
        if let Err(error) = self.secrets.reauthorize(&run).await {
            return self
                .fail_with_resources(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    None,
                    &error.to_string(),
                )
                .await;
        }
        let instance = match self.provider.provision(spec).await {
            Ok(instance) => instance,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        attachment.as_ref().map(|value| &value.lease),
                        None,
                        &error.to_string(),
                    )
                    .await;
            }
        };
        self.active
            .lock()
            .await
            .insert(command.run_id, Arc::clone(&instance));
        if self
            .repository
            .get(command.run_id)
            .await?
            .cancel_requested_at
            .is_some()
        {
            self.repository
                .transition(command.run_id, RunState::Cancelled, None, None)
                .await?;
            return self
                .cleanup(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    Some(instance),
                )
                .await;
        }
        if let Err(error) = self
            .repository
            .transition(command.run_id, RunState::Starting, None, None)
            .await
        {
            self.abort_vm_keep_lease(command.run_id, &instance).await;
            return Err(error.into());
        }
        let mut events = instance.subscribe_events();
        if let Err(error) = instance.start().await {
            let cancelled = self
                .repository
                .get(command.run_id)
                .await?
                .cancel_requested_at
                .is_some();
            if cancelled {
                self.repository
                    .transition(command.run_id, RunState::Cancelled, None, None)
                    .await?;
                return self
                    .cleanup(
                        command.run_id,
                        attachment.as_ref().map(|value| &value.lease),
                        Some(instance),
                    )
                    .await;
            }
            return self
                .fail_with_resources(
                    command.run_id,
                    attachment.as_ref().map(|value| &value.lease),
                    Some(instance),
                    &error.to_string(),
                )
                .await;
        }
        let mut lease = match attachment.as_ref() {
            Some(attachment) => match self.volumes.mark_attached(&attachment.lease).await {
                Ok(lease) => Some(lease),
                Err(error) => {
                    self.abort_vm_keep_lease(command.run_id, &instance).await;
                    return Err(error.into());
                }
            },
            None => None,
        };
        if self
            .repository
            .get(command.run_id)
            .await?
            .cancel_requested_at
            .is_some()
        {
            self.repository
                .transition(command.run_id, RunState::Cancelled, None, None)
                .await?;
            return self
                .cleanup(command.run_id, lease.as_ref(), Some(instance))
                .await;
        }
        if let Err(error) = self
            .repository
            .transition(command.run_id, RunState::Running, None, None)
            .await
        {
            self.abort_vm_keep_lease(command.run_id, &instance).await;
            return Err(error.into());
        }

        let completion_result = async {
            self.wait_and_persist_events(command.run_id, &instance, &mut events, lease.as_mut())
                .await
        };
        let completion = match execution_timeout {
            Some(limit) => {
                if let Ok(result) = tokio::time::timeout(limit, completion_result).await {
                    result
                } else {
                    instance.stop(StopMode::Force).await?;
                    return self
                        .fail_with_resources(
                            command.run_id,
                            lease.as_ref(),
                            Some(instance),
                            "guest wall-clock timeout elapsed",
                        )
                        .await;
                }
            }
            None => completion_result.await,
        };
        let completion = match completion {
            Ok(completion) => completion,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        lease.as_ref(),
                        Some(instance),
                        &error.to_string(),
                    )
                    .await;
            }
        };
        instance.destroy().await?;
        self.active.lock().await.remove(&command.run_id);
        self.secrets.destroy_after_guest(command.run_id).await?;
        let current = self.repository.get(command.run_id).await?;
        let mut result_failure = None;
        if current.cancel_requested_at.is_some() {
            self.workspaces.abandon(command.run_id).await?;
        } else if workspace_enabled {
            if let Some(message) = completion.finalize_message.as_deref() {
                if let Err(error) = self.workspaces.finalize(&current, message).await {
                    result_failure = Some(error.to_string());
                }
            } else {
                result_failure = Some(String::from(
                    "guest exited without finalizing its repository workspace",
                ));
                self.workspaces.abandon(command.run_id).await?;
            }
        }
        let outcome = if current.cancel_requested_at.is_some() {
            RunState::Cancelled
        } else if result_failure.is_some() {
            RunState::Failed
        } else if completion.finalize_message.is_some()
            || (completion.exit.code == Some(0) && completion.exit.signal.is_none())
        {
            RunState::Succeeded
        } else {
            RunState::Failed
        };
        self.repository
            .transition(
                command.run_id,
                outcome,
                Some(&completion.exit),
                result_failure.as_deref(),
            )
            .await?;
        self.cleanup(command.run_id, lease.as_ref(), None).await
    }

    /// Records a cancellation command and stops the active VM when present.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence or VM shutdown fails.
    pub async fn cancel_run(&self, command: &CancelRun) -> Result<bool, OrchestratorError> {
        let inserted = self.repository.request_cancel(command).await?;
        if !inserted {
            return Ok(false);
        }
        let instance = self.active.lock().await.get(&command.run_id).cloned();
        if let Some(instance) = instance {
            instance
                .stop(StopMode::Graceful {
                    timeout: self.cancellation_timeout,
                })
                .await?;
        }
        Ok(true)
    }

    /// Reconciles expired leases after a supervisor restart.
    ///
    /// Provider orphan cleanup must succeed before the volume store releases
    /// the fenced lease.
    ///
    /// # Errors
    ///
    /// Returns an error without releasing the affected lease when provider
    /// cleanup cannot be confirmed.
    pub async fn recover_stale_leases(&self) -> Result<usize, OrchestratorError> {
        let leases = self.volumes.stale_leases(OffsetDateTime::now_utc()).await?;
        let mut recovered = 0;
        for lease in leases {
            self.volumes.begin_recovery(&lease).await?;
            let run = self.repository.get(lease.run_id).await?;
            let vm_id = run
                .vm_id
                .as_deref()
                .map_or_else(|| VmId(run.id.to_string()), |value| VmId(value.to_owned()));
            self.provider.cleanup_orphan(&vm_id).await?;
            self.volumes.finish_recovery(&lease).await?;
            self.finish_recovered_run(run).await?;
            recovered += 1;
        }
        Ok(recovered)
    }

    /// Reconciles durable state after the supervisor starts.
    ///
    /// Expired active leases are fenced first. Runs left after a confirmed
    /// lease release are then finalized, closing the crash window between
    /// volume release and the final `CleanedUp` database transition. Queued
    /// and pre-lease runs remain available for command redelivery.
    ///
    /// # Errors
    ///
    /// Returns an error without claiming cleanup when provider or durable
    /// reconciliation cannot be confirmed.
    pub async fn recover_after_restart(&self) -> Result<usize, OrchestratorError> {
        let mut recovered = self.workspaces.recover().await?;
        recovered += self.runtimes.recover().await?;
        recovered += self.secrets.recover().await?;
        recovered += self.recover_stale_leases().await?;
        for run in self.repository.recoverable_runs().await? {
            if matches!(run.state, RunState::Queued | RunState::LeasingVolume) {
                continue;
            }
            if self.volumes.active_lease_for_run(run.id).await?.is_some() {
                continue;
            }
            let vm_id = run
                .vm_id
                .as_deref()
                .map_or_else(|| VmId(run.id.to_string()), |value| VmId(value.to_owned()));
            self.provider.cleanup_orphan(&vm_id).await?;
            self.finish_recovered_run(run).await?;
            recovered += 1;
        }
        recovered += self.completion.recover().await?;
        Ok(recovered)
    }

    async fn build_spec(
        &self,
        run: &Run,
        attachment: Option<&VolumeAttachment>,
        workspace_mounts: Vec<vm_trait::VmMount>,
    ) -> Result<VmSpec, VmError> {
        let mut spec = self.spec_factory.build(run).await?;
        spec.id = VmId(run.id.to_string());
        spec.disks.retain(|disk| disk.id != INSTANCE_STATE_DISK_ID);
        if let Some(attachment) = attachment {
            spec.disks.push(VmDisk {
                id: attachment.disk_id.to_owned(),
                host_path: attachment.volume.host_path.clone(),
                format: DiskFormat::Raw,
                read_only: false,
            });
            spec.labels.insert(
                String::from("hephaestus.agent-state.filesystem-uuid"),
                attachment.volume.filesystem_uuid.to_string(),
            );
            spec.labels.insert(
                String::from("hephaestus.agent-state.mount-path"),
                String::from("/var/lib/hephaestus"),
            );
        }
        spec.mounts.extend(workspace_mounts);
        Ok(spec)
    }

    async fn wait_and_persist_events(
        &self,
        run_id: RunId,
        instance: &Arc<dyn VmInstance>,
        events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
        mut lease: Option<&mut VolumeLease>,
    ) -> Result<GuestCompletion, OrchestratorError> {
        let wait = instance.wait();
        tokio::pin!(wait);
        let heartbeat_period = lease.as_deref().map_or(Duration::from_secs(3600), |lease| {
            Duration::try_from((lease.expires_at - lease.heartbeat_at) / 2)
                .unwrap_or_else(|_| Duration::from_secs(1))
                .max(Duration::from_millis(1))
        });
        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + heartbeat_period,
            heartbeat_period,
        );
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut events_open = true;
        let mut finalize_message = None;
        let mut finalize_stop_requested = false;
        loop {
            tokio::select! {
                result = &mut wait => {
                    let exit = result?;
                    self.drain_vm_events(run_id, events, &mut finalize_message)
                        .await?;
                    return Ok(GuestCompletion {
                        exit,
                        finalize_message,
                    });
                },
                _ = heartbeat.tick(), if lease.is_some() => {
                    let current = lease.as_deref_mut().expect("lease branch is guarded");
                    *current = self.volumes.heartbeat(current).await?;
                }
                event = events.recv(), if events_open => {
                    match event {
                        Ok(event) => {
                            capture_finalize(&event, &mut finalize_message);
                            self.persist_vm_event(run_id, event).await?;
                            if finalize_message.is_some() && !finalize_stop_requested {
                                finalize_stop_requested = true;
                                instance.stop(StopMode::Force).await?;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            self.persist_lagged_event(run_id, skipped).await?;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            events_open = false;
                        }
                    }
                }
            }
        }
    }

    async fn drain_vm_events(
        &self,
        run_id: RunId,
        events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
        finalize_message: &mut Option<String>,
    ) -> Result<(), OrchestratorError> {
        loop {
            match events.try_recv() {
                Ok(event) => {
                    capture_finalize(&event, finalize_message);
                    self.persist_vm_event(run_id, event).await?;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                    self.persist_lagged_event(run_id, skipped).await?;
                }
                Err(
                    tokio::sync::broadcast::error::TryRecvError::Empty
                    | tokio::sync::broadcast::error::TryRecvError::Closed,
                ) => return Ok(()),
            }
        }
    }

    async fn persist_vm_event(
        &self,
        run_id: RunId,
        event: VmEvent,
    ) -> Result<(), OrchestratorError> {
        self.repository
            .append_vm_event(run_id, stored_event(event))
            .await
            .map_err(Into::into)
    }

    async fn persist_lagged_event(
        &self,
        run_id: RunId,
        skipped: u64,
    ) -> Result<(), OrchestratorError> {
        self.repository
            .append_vm_event(
                run_id,
                StoredVmEvent {
                    event_type: String::from("vm.events_lagged"),
                    payload: json!({"skipped": skipped}),
                    occurred_at: OffsetDateTime::now_utc(),
                },
            )
            .await
            .map_err(Into::into)
    }

    async fn fail_without_lease(
        &self,
        run_id: RunId,
        failure: &str,
    ) -> Result<Run, OrchestratorError> {
        self.repository
            .transition(run_id, RunState::Failed, None, Some(failure))
            .await?;
        self.repository
            .transition(run_id, RunState::CleaningUp, None, None)
            .await?;
        let cleaned = self
            .repository
            .transition(run_id, RunState::CleanedUp, None, None)
            .await?;
        self.completion.after_cleanup(&cleaned).await?;
        Ok(cleaned)
    }

    async fn fail_with_resources(
        &self,
        run_id: RunId,
        lease: Option<&VolumeLease>,
        instance: Option<Arc<dyn VmInstance>>,
        failure: &str,
    ) -> Result<Run, OrchestratorError> {
        self.repository
            .transition(run_id, RunState::Failed, None, Some(failure))
            .await?;
        self.cleanup(run_id, lease, instance).await
    }

    async fn cancel_before_vm(
        &self,
        run: Run,
        lease: Option<&VolumeLease>,
    ) -> Result<Run, OrchestratorError> {
        self.repository
            .transition(run.id, RunState::Cancelled, None, None)
            .await?;
        self.cleanup(run.id, lease, None).await
    }

    async fn cleanup(
        &self,
        run_id: RunId,
        lease: Option<&VolumeLease>,
        instance: Option<Arc<dyn VmInstance>>,
    ) -> Result<Run, OrchestratorError> {
        if let Err(error) = self
            .repository
            .transition(run_id, RunState::CleaningUp, None, None)
            .await
        {
            if let Some(instance) = instance.as_ref() {
                self.abort_vm_keep_lease(run_id, instance).await;
            }
            return Err(error.into());
        }
        if let Some(instance) = instance {
            instance.destroy().await?;
        }
        self.active.lock().await.remove(&run_id);
        self.secrets.destroy_after_guest(run_id).await?;
        self.runtimes.destroy(run_id).await?;
        self.workspaces.abandon(run_id).await?;
        if let Some(lease) = lease {
            self.volumes.release_after_detach(lease).await?;
        }
        let cleaned = self
            .repository
            .transition(run_id, RunState::CleanedUp, None, None)
            .await?;
        self.completion.after_cleanup(&cleaned).await?;
        Ok(cleaned)
    }

    async fn abort_vm_keep_lease(&self, run_id: RunId, instance: &Arc<dyn VmInstance>) {
        if let Err(error) = instance.destroy().await {
            tracing::error!(%run_id, %error, "failed to destroy VM after orchestration error");
        }
        self.active.lock().await.remove(&run_id);
    }

    async fn finish_recovered_run(&self, run: Run) -> Result<(), OrchestratorError> {
        self.secrets.destroy_after_guest(run.id).await?;
        self.runtimes.destroy(run.id).await?;
        self.workspaces.abandon(run.id).await?;
        let run = match run.state {
            RunState::Succeeded | RunState::Failed | RunState::Cancelled | RunState::CleaningUp => {
                run
            }
            _ => {
                self.repository
                    .transition(
                        run.id,
                        RunState::Failed,
                        None,
                        Some("supervisor restarted while run resources were active"),
                    )
                    .await?
            }
        };
        let run = if run.state == RunState::CleaningUp {
            run
        } else {
            self.repository
                .transition(run.id, RunState::CleaningUp, None, None)
                .await?
        };
        let cleaned = self
            .repository
            .transition(run.id, RunState::CleanedUp, None, None)
            .await?;
        self.completion.after_cleanup(&cleaned).await?;
        Ok(())
    }
}

fn stored_event(event: VmEvent) -> StoredVmEvent {
    let (event_type, payload) = match event {
        VmEvent::Started { ingress } => (
            "vm.started",
            json!({"ingress": ingress.into_iter().map(|forward| json!({
                "protocol": format!("{:?}", forward.protocol),
                "bind_addr": forward.bind_addr,
                "host_port": forward.host_port,
                "guest_port": forward.guest_port
            })).collect::<Vec<_>>() }),
        ),
        VmEvent::Ready => ("vm.ready", json!({})),
        VmEvent::Log { stream, bytes } => (
            "vm.log",
            json!({"stream": format!("{stream:?}"), "bytes": bytes}),
        ),
        VmEvent::Metric(metric) => (
            "vm.metric",
            json!({"name": metric.name, "value": metric.value, "labels": metric.labels}),
        ),
        VmEvent::FinalizeResult { message } => ("vm.finalize_result", json!({"message": message})),
        VmEvent::Exited(exit) => (
            "vm.exited",
            json!({"code": exit.code, "signal": exit.signal}),
        ),
        _ => ("vm.unknown", json!({})),
    };
    StoredVmEvent {
        event_type: event_type.to_owned(),
        payload,
        occurred_at: OffsetDateTime::now_utc(),
    }
}

/// Durable orchestration failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OrchestratorError {
    /// A duplicate delivery observed work that still owns or may own runtime
    /// resources and must be retried after reconciliation.
    #[error("run {0} is still in progress")]
    RunInProgress(RunId),
    /// Run persistence failed.
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    /// Persistent-volume operation failed.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// VM lifecycle operation failed.
    #[error(transparent)]
    Vm(#[from] VmError),
    /// Repository workspace or result publication failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Exact release-artifact or host-context lifecycle failed.
    #[error(transparent)]
    Runtime(#[from] RunRuntimeError),
    /// Exact secret dispatch or ephemeral cleanup failed.
    #[error(transparent)]
    Secret(#[from] RunSecretError),
    /// Post-cleanup domain result processing failed.
    #[error(transparent)]
    Completion(#[from] RunCompletionError),
}

struct GuestCompletion {
    exit: VmExit,
    finalize_message: Option<String>,
}

fn capture_finalize(event: &VmEvent, message: &mut Option<String>) {
    if let VmEvent::FinalizeResult { message: finalized } = event {
        *message = Some(finalized.clone());
    }
}
