use run_domain::Run;
use runtime_types::RunId;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::{DiskFormat, StopMode, VmDisk, VmError, VmEvent, VmId, VmInstance, VmSpec};
use volume_trait::{INSTANCE_STATE_DISK_ID, VolumeAttachment, VolumeLease};

use super::{
    OrchestratorError, RunOrchestrator,
    authority::RunAuthorityError,
    errors::{GuestCompletion, capture_finalize, stored_event},
};
use crate::StoredVmEvent;

impl RunOrchestrator {
    pub(super) async fn build_spec(
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

    pub(super) async fn wait_and_persist_events(
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

    pub(super) async fn await_runtime_authority_acknowledgement(
        &self,
        run: &Run,
        events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
        expected: (Uuid, u64),
        timeout_limit: Duration,
    ) -> Result<(), RunAuthorityError> {
        tokio::time::timeout(timeout_limit, async {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let acknowledgement = match &event {
                            VmEvent::RuntimeAuthorityAcknowledged {
                                session_id,
                                generation,
                            } => Some((*session_id, *generation)),
                            _ => None,
                        };
                        self.persist_vm_event(run.id, event)
                            .await
                            .map_err(|_| RunAuthorityError::redacted("event persistence failed"))?;
                        if let Some(actual) = acknowledgement {
                            if actual != expected {
                                return Err(RunAuthorityError::redacted(
                                    "guest acknowledged a different runtime authority",
                                ));
                            }
                            return self.authority.acknowledge(run, actual.0, actual.1).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        self.persist_lagged_event(run.id, skipped)
                            .await
                            .map_err(|_| RunAuthorityError::redacted("event persistence failed"))?;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(RunAuthorityError::redacted(
                            "guest authority acknowledgement channel closed",
                        ));
                    }
                }
            }
        })
        .await
        .map_err(|_| RunAuthorityError::redacted("guest authority acknowledgement timed out"))?
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
}
