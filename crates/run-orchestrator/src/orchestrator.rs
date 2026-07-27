use run_domain::{CancelRun, Run, RunState, StartRun};
use runtime_types::RunId;
use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use vm_trait::{
    DiskFormat, StopMode, VmDisk, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec,
};
use volume_trait::{AGENT_STATE_DISK_ID, VolumeAttachment, VolumeError, VolumeLease, VolumeStore};

use crate::{RepositoryError, RunRepository, StoredVmEvent};

/// Builds the non-volume portion of a VM specification for a run.
pub trait VmSpecFactory: Send + Sync + 'static {
    /// Creates a VM specification. The orchestrator replaces any disk named
    /// `agent-state` with the currently leased attachment.
    ///
    /// # Errors
    ///
    /// Returns an error when run configuration cannot produce a valid spec.
    fn build(&self, run: &Run) -> Result<VmSpec, VmError>;
}

/// Durable coordinator for run, volume, and VM lifecycles.
pub struct RunOrchestrator {
    repository: Arc<dyn RunRepository>,
    volumes: Arc<dyn VolumeStore>,
    provider: Arc<dyn VmProvider>,
    spec_factory: Arc<dyn VmSpecFactory>,
    active: Mutex<HashMap<RunId, Arc<dyn VmInstance>>>,
    agent_state_capacity_bytes: u64,
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
        agent_state_capacity_bytes: u64,
    ) -> Self {
        Self {
            repository,
            volumes,
            provider,
            spec_factory,
            active: Mutex::new(HashMap::new()),
            agent_state_capacity_bytes,
            cancellation_timeout: Duration::from_secs(10),
        }
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
                RunState::CleanedUp => return Ok(created.run),
                _ => return Err(OrchestratorError::RunInProgress(command.run_id)),
            }
        }
        if created.run.state == RunState::Queued {
            self.repository
                .transition(command.run_id, RunState::LeasingVolume, None, None)
                .await?;
        }

        let volume = match self
            .volumes
            .resolve_agent_state(command.agent_id, self.agent_state_capacity_bytes)
            .await
        {
            Ok(volume) => volume,
            Err(error) => {
                return self
                    .fail_without_lease(command.run_id, &error.to_string())
                    .await;
            }
        };
        let attachment = match self.volumes.acquire(volume.id, command.run_id).await {
            Ok(attachment) => attachment,
            Err(error) => {
                return self
                    .fail_without_lease(command.run_id, &error.to_string())
                    .await;
            }
        };
        let vm_id = VmId(command.run_id.to_string());
        let run = self
            .repository
            .bind_resources(command.run_id, volume.id, attachment.lease.id, &vm_id.0)
            .await?;
        if run.cancel_requested_at.is_some() {
            return self.cancel_before_vm(run, &attachment.lease).await;
        }

        self.repository
            .transition(command.run_id, RunState::Provisioning, None, None)
            .await?;
        let spec = match self.build_spec(&run, &attachment) {
            Ok(spec) => spec,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        &attachment.lease,
                        None,
                        &error.to_string(),
                    )
                    .await;
            }
        };
        let instance = match self.provider.provision(spec).await {
            Ok(instance) => instance,
            Err(error) => {
                return self
                    .fail_with_resources(
                        command.run_id,
                        &attachment.lease,
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
                .cleanup(command.run_id, &attachment.lease, Some(instance))
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
                    .cleanup(command.run_id, &attachment.lease, Some(instance))
                    .await;
            }
            return self
                .fail_with_resources(
                    command.run_id,
                    &attachment.lease,
                    Some(instance),
                    &error.to_string(),
                )
                .await;
        }
        let mut lease = match self.volumes.mark_attached(&attachment.lease).await {
            Ok(lease) => lease,
            Err(error) => {
                self.abort_vm_keep_lease(command.run_id, &instance).await;
                return Err(error.into());
            }
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
            return self.cleanup(command.run_id, &lease, Some(instance)).await;
        }
        if let Err(error) = self
            .repository
            .transition(command.run_id, RunState::Running, None, None)
            .await
        {
            self.abort_vm_keep_lease(command.run_id, &instance).await;
            return Err(error.into());
        }

        let exit = match self
            .wait_and_persist_events(command.run_id, &instance, &mut events, &mut lease)
            .await
        {
            Ok(exit) => exit,
            Err(error) => {
                return self
                    .fail_with_resources(command.run_id, &lease, Some(instance), &error.to_string())
                    .await;
            }
        };
        let current = self.repository.get(command.run_id).await?;
        let outcome = if current.cancel_requested_at.is_some() {
            RunState::Cancelled
        } else if exit.code == Some(0) && exit.signal.is_none() {
            RunState::Succeeded
        } else {
            RunState::Failed
        };
        self.repository
            .transition(command.run_id, outcome, Some(&exit), None)
            .await?;
        self.cleanup(command.run_id, &lease, Some(instance)).await
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
        let mut recovered = self.recover_stale_leases().await?;
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
        Ok(recovered)
    }

    fn build_spec(&self, run: &Run, attachment: &VolumeAttachment) -> Result<VmSpec, VmError> {
        let mut spec = self.spec_factory.build(run)?;
        spec.id = VmId(run.id.to_string());
        spec.disks.retain(|disk| disk.id != AGENT_STATE_DISK_ID);
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
        Ok(spec)
    }

    async fn wait_and_persist_events(
        &self,
        run_id: RunId,
        instance: &Arc<dyn VmInstance>,
        events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
        lease: &mut VolumeLease,
    ) -> Result<VmExit, OrchestratorError> {
        let wait = instance.wait();
        tokio::pin!(wait);
        let lease_duration = lease.expires_at - lease.heartbeat_at;
        let heartbeat_period = Duration::try_from(lease_duration / 2)
            .unwrap_or_else(|_| Duration::from_secs(1))
            .max(Duration::from_millis(1));
        let mut heartbeat = tokio::time::interval(heartbeat_period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut events_open = true;
        loop {
            tokio::select! {
                result = &mut wait => {
                    let exit = result?;
                    self.drain_vm_events(run_id, events).await?;
                    return Ok(exit);
                },
                _ = heartbeat.tick() => {
                    *lease = self.volumes.heartbeat(lease).await?;
                }
                event = events.recv(), if events_open => {
                    match event {
                        Ok(event) => {
                            self.persist_vm_event(run_id, event).await?;
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
    ) -> Result<(), OrchestratorError> {
        loop {
            match events.try_recv() {
                Ok(event) => self.persist_vm_event(run_id, event).await?,
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
        self.repository
            .transition(run_id, RunState::CleanedUp, None, None)
            .await
            .map_err(Into::into)
    }

    async fn fail_with_resources(
        &self,
        run_id: RunId,
        lease: &VolumeLease,
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
        lease: &VolumeLease,
    ) -> Result<Run, OrchestratorError> {
        self.repository
            .transition(run.id, RunState::Cancelled, None, None)
            .await?;
        self.cleanup(run.id, lease, None).await
    }

    async fn cleanup(
        &self,
        run_id: RunId,
        lease: &VolumeLease,
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
        self.volumes.release_after_detach(lease).await?;
        self.repository
            .transition(run_id, RunState::CleanedUp, None, None)
            .await
            .map_err(Into::into)
    }

    async fn abort_vm_keep_lease(&self, run_id: RunId, instance: &Arc<dyn VmInstance>) {
        if let Err(error) = instance.destroy().await {
            tracing::error!(%run_id, %error, "failed to destroy VM after orchestration error");
        }
        self.active.lock().await.remove(&run_id);
    }

    async fn finish_recovered_run(&self, run: Run) -> Result<(), OrchestratorError> {
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
        self.repository
            .transition(run.id, RunState::CleanedUp, None, None)
            .await?;
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
}
