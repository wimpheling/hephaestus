use run_domain::Run;
use runtime_types::RunId;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use vm_trait::{VmError, VmId, VmProviderOwnerScope};
use volume_trait::VolumeError;

use super::{
    OrchestratorError, RunOrchestrator, plural_evidence::PreparedRunVolumes, prepare::PreparedStart,
};

/// Private execution state; neither VM exit nor handle absence confirms cleanup.
#[derive(Default)]
pub struct GuestCleanupPhase {
    pub draining: AtomicBool,
    pub confirmed: AtomicBool,
}

impl RunOrchestrator {
    pub(super) async fn execute_monitored_run(
        &self,
        run_id: RunId,
        prepared: PreparedStart,
    ) -> Result<Run, OrchestratorError> {
        let run = prepared.run.clone();
        let mut monitored = prepared
            .plural
            .as_ref()
            .ok_or(VolumeError::InvalidState("plural preparation is absent"))?
            .clone();
        let scope = self.provider.owner_scope()?;
        let vm = VmId(
            run.vm_id
                .clone()
                .ok_or(VmError::InvalidState("planned VM identity is absent"))?,
        );
        // Prove that this captured actual owner is the durable plan before IO.
        self.canonical_cleanup
            .as_ref()
            .ok_or(VolumeError::InvalidState("canonical cleanup is absent"))?
            .repository
            .bind_vm_before_provision(
                run.id,
                run.instance_id,
                run.instance_revision_id,
                &super::canonical_cleanup::host(&scope)?,
                &vm,
            )
            .await?;
        let phase = GuestCleanupPhase::default();
        let failure = {
            let execution = self.execute_prepared_run(run_id, prepared, Some(&phase));
            tokio::pin!(execution);
            tokio::select! {
                result = &mut execution => match result {
                    Ok(run) => return Ok(run),
                    Err(error) => error,
                },
                failure = self.monitor_volume_execution(&run, &mut monitored, &phase) => failure,
            }
            // Drop the execution future before taking its physical operation guard.
            // No cancelled provision/Start/event future can resume after closure.
        };
        self.fail_monitored_run(
            &run,
            &scope,
            &vm,
            failure,
            phase.confirmed.load(Ordering::SeqCst),
        )
        .await
    }

    async fn monitor_volume_execution(
        &self,
        run: &Run,
        volumes: &mut PreparedRunVolumes,
        phase: &GuestCleanupPhase,
    ) -> OrchestratorError {
        let period = Duration::from_secs(1);
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if phase.confirmed.load(Ordering::SeqCst) {
                return std::future::pending().await;
            }
            let checked = tokio::time::timeout(Duration::from_secs(2), async {
                if phase.draining.load(Ordering::SeqCst) {
                    self.check_draining_authority(run, &volumes.selections)
                        .await
                } else {
                    let result = self.refresh_run_volumes(run, volumes, false).await;
                    // Own positive drain may have begun during a refresh. Its
                    // deliberate closure is not authority withdrawal. Recheck
                    // live sources within the SAME absolute check deadline.
                    if result.is_err() && phase.draining.load(Ordering::SeqCst) {
                        self.check_draining_authority(run, &volumes.selections)
                            .await
                    } else {
                        result
                    }
                }
            })
            .await
            .unwrap_or_else(|_| {
                Err(VolumeError::InvalidState(
                    "combined live volume authorization deadline elapsed",
                )
                .into())
            });
            if let Err(error) = checked {
                if phase.confirmed.load(Ordering::SeqCst) {
                    return std::future::pending().await;
                }
                return error;
            }
        }
    }

    async fn fail_monitored_run(
        &self,
        run: &Run,
        scope: &VmProviderOwnerScope,
        vm: &VmId,
        failure: OrchestratorError,
        guest_confirmed: bool,
    ) -> Result<Run, OrchestratorError> {
        // Physical revocation does not wait for an unresponsive event/database
        // path. It creates no durable receipt and never releases a lease itself.
        if !guest_confirmed && let Err(cleanup) = self.stop_monitored_guest(run.id, scope, vm).await
        {
            return Err(incomplete(&failure, cleanup));
        }
        match tokio::time::timeout(
            Duration::from_secs(4),
            self.fail_claimed_run(run.id, &failure.to_string()),
        )
        .await
        {
            Ok(Ok(_)) => Err(failure),
            Ok(Err(cleanup)) => Err(incomplete(&failure, cleanup)),
            Err(_) => Err(incomplete(
                &failure,
                VolumeError::InvalidState("durable cleanup bookkeeping deadline elapsed").into(),
            )),
        }
    }

    async fn stop_monitored_guest(
        &self,
        run_id: RunId,
        scope: &VmProviderOwnerScope,
        vm: &VmId,
    ) -> Result<(), OrchestratorError> {
        tokio::time::timeout(self.cleanup_timeout, async {
            let operation = self.lock_run_operation(run_id).await?;
            if self.provider.owner_scope()? != *scope {
                return Err(
                    VmError::InvalidState("provider owner differs from captured run plan").into(),
                );
            }
            let active = self.active.lock().await.get(&run_id).cloned();
            if let Some(instance) = active.as_ref() {
                if instance.id() != vm {
                    return Err(VmError::InvalidState(
                        "active handle differs from captured planned VM",
                    )
                    .into());
                }
                instance.destroy().await?;
            }
            self.provider.cleanup_orphan_scoped(scope, vm).await?;
            // Scoped provider confirmation is required even after handle.destroy.
            self.active.lock().await.remove(&run_id);
            drop(active);
            drop(operation);
            Ok::<(), OrchestratorError>(())
        })
        .await
        .map_err(|_| VmError::InvalidState("physical revocation deadline elapsed"))?
    }
}

fn incomplete(failure: &OrchestratorError, cleanup: OrchestratorError) -> OrchestratorError {
    OrchestratorError::CleanupIncomplete {
        failure: failure.to_string(),
        cleanup: Box::new(cleanup),
    }
}
