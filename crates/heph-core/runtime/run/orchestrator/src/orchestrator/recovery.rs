use run_domain::{CancelRun, RunState};
use time::OffsetDateTime;
use vm_trait::{StopMode, VmId};

use super::{OrchestratorError, RunOrchestrator};

impl RunOrchestrator {
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
        recovered += self.authority.recover().await?;
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
        // Runtime-Git mounts are removed only after stale VM cleanup above has
        // stopped every possible guest that could still hold them.
        recovered += self.runtime_git_workspace.recover_runtime_git().await?;
        Ok(recovered)
    }
}
