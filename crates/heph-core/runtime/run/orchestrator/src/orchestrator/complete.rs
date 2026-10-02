use run_domain::RunState;

use super::{OrchestratorError, RunOrchestrator, provision::StartedRun};

pub async fn complete_started_run(
    orchestrator: &RunOrchestrator,
    run_id: runtime_types::RunId,
    started: StartedRun,
    guest_cleanup: Option<&super::plural_monitor::GuestCleanupPhase>,
) -> Result<run_domain::Run, OrchestratorError> {
    let StartedRun {
        run: _run,
        attachment: _attachment,
        workspace_enabled,
        instance,
        mut events,
        mut lease,
        plural: _plural,
        execution_timeout,
    } = started;
    let completion_result = async {
        orchestrator
            .wait_and_persist_events(run_id, &instance, &mut events, lease.as_mut())
            .await
    };
    let completion = match execution_timeout {
        Some(limit) => {
            if let Ok(result) = tokio::time::timeout(limit, completion_result).await {
                result
            } else {
                if orchestrator.canonical_cleanup.is_none() {
                    instance.stop(vm_trait::StopMode::Force).await?;
                }
                return orchestrator
                    .fail_with_resources(
                        run_id,
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
            return orchestrator
                .fail_with_resources(run_id, lease.as_ref(), Some(instance), &error.to_string())
                .await;
        }
    };
    // Enter an owned drain only after successful authoritative guest completion.
    // The live-start claim still excludes future starts for this run. No fence
    // is released, and live source/caller checks continue until confirmation.
    if let Some(phase) = guest_cleanup {
        phase
            .draining
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    cleanup_completed_guest(orchestrator, run_id, instance, guest_cleanup).await?;
    let finish = finish_completed_guest(
        orchestrator,
        run_id,
        workspace_enabled,
        lease.as_ref(),
        &completion,
    );
    if guest_cleanup.is_some() {
        tokio::time::timeout(std::time::Duration::from_secs(4), finish)
            .await
            .map_err(|_| OrchestratorError::CleanupIncomplete {
                failure: String::from("guest completion bookkeeping"),
                cleanup: Box::new(
                    volume_trait::VolumeError::InvalidState(
                        "completion bookkeeping deadline elapsed",
                    )
                    .into(),
                ),
            })?
    } else {
        finish.await
    }
}

async fn finish_completed_guest(
    orchestrator: &RunOrchestrator,
    run_id: runtime_types::RunId,
    workspace_enabled: bool,
    lease: Option<&volume_trait::VolumeLease>,
    completion: &super::errors::GuestCompletion,
) -> Result<run_domain::Run, OrchestratorError> {
    orchestrator.authority.revoke_after_guest(run_id).await?;
    orchestrator.secrets.destroy_after_guest(run_id).await?;
    let current = orchestrator.repository.get(run_id).await?;
    let mut result_failure = None;
    if current.cancel_requested_at.is_some() {
        orchestrator.workspaces.abandon(run_id).await?;
    } else if workspace_enabled {
        if let Some(message) = completion.finalize_message.as_deref() {
            if let Err(error) = orchestrator.workspaces.finalize(&current, message).await {
                result_failure = Some(error.to_string());
            }
        } else {
            result_failure = Some(String::from(
                "guest exited without finalizing its repository workspace",
            ));
            orchestrator.workspaces.abandon(run_id).await?;
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
    orchestrator
        .repository
        .transition(
            run_id,
            outcome,
            Some(&completion.exit),
            result_failure.as_deref(),
        )
        .await?;
    orchestrator.cleanup(run_id, lease, None).await
}

async fn cleanup_completed_guest(
    orchestrator: &RunOrchestrator,
    run_id: runtime_types::RunId,
    instance: std::sync::Arc<dyn vm_trait::VmInstance>,
    guest_cleanup: Option<&super::plural_monitor::GuestCleanupPhase>,
) -> Result<(), OrchestratorError> {
    if orchestrator.canonical_cleanup.is_some() {
        orchestrator
            .confirm_canonical_guest_cleanup(run_id, Some(instance))
            .await
            .map_err(|cleanup| OrchestratorError::CleanupIncomplete {
                failure: String::from("guest completion cleanup"),
                cleanup: Box::new(cleanup),
            })?;
    } else {
        instance.destroy().await?;
        orchestrator.active.lock().await.remove(&run_id);
    }
    // A positive provider-confirmed observation stops the independent monitor.
    // In-memory handle absence alone never sets this flag.
    if let Some(phase) = guest_cleanup {
        phase
            .confirmed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}
