//! Strict scalar release remains the qualified trusted provider callback boundary.
use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};
use run_domain::{Run, RunState};
use runtime_types::RunId;
use std::sync::Arc;
use vm_trait::{VmExit, VmInstance};
use volume_trait::{ScalarLeaseHistory, VolumeLease};

impl RunOrchestrator {
    pub(super) async fn record_legacy_destroy_failure(
        &self,
        run: RunId,
        exit: &VmExit,
        error: OrchestratorError,
    ) -> Result<Run, OrchestratorError> {
        let message = error.to_string();
        let recording = tokio::time::timeout(std::time::Duration::from_secs(4), async {
            self.repository
                .transition(run, RunState::Failed, Some(exit), Some(&message))
                .await?;
            self.transition_legacy_cleanup(run, RunState::CleaningUp, None, None)
                .await?;
            Ok::<(), OrchestratorError>(())
        })
        .await;
        let cleanup = match recording {
            Ok(Ok(())) => error,
            Ok(Err(recording)) => recording,
            Err(_) => invalid("Legacy failed destruction recording deadline elapsed"),
        };
        Err(OrchestratorError::CleanupIncomplete {
            failure: message,
            cleanup: Box::new(cleanup),
        })
    }
    pub(super) async fn cleanup_legacy(
        &self,
        run: RunId,
        instance: Option<Arc<dyn VmInstance>>,
        failure: Option<(&str, Option<&VmExit>)>,
    ) -> Result<Run, OrchestratorError> {
        let deadline = tokio::time::Instant::now() + self.cleanup_timeout;
        let current = self.read_legacy_run(run).await;
        if current
            .as_ref()
            .is_ok_and(|r| r.state == RunState::CleanedUp)
        {
            return self.legacy_terminal(run).await;
        }
        // Even a failed initial read must not prevent exact cached physical fallback.
        let physical = self
            .close_confirm_legacy_until(run, instance, deadline)
            .await;
        let current = self.read_legacy_run(run).await?;
        if let Some((message, exit)) = failure {
            if !matches!(
                current.state,
                RunState::Succeeded | RunState::Failed | RunState::Cancelled | RunState::CleaningUp
            ) {
                let outcome = if current.cancel_requested_at.is_some() {
                    RunState::Cancelled
                } else {
                    RunState::Failed
                };
                self.transition_legacy_cleanup(run, outcome, exit, Some(message))
                    .await?;
            }
        }
        let current = self.read_legacy_run(run).await?;
        if current.state != RunState::CleaningUp {
            self.transition_legacy_cleanup(run, RunState::CleaningUp, None, None)
                .await?;
        }
        // Outcome/closure remain truthful on physical failure; release is forbidden.
        physical?;
        let history = self.volumes.scalar_lease_history(run).await?;
        let held = match &history {
            ScalarLeaseHistory::Held(lease)
            | ScalarLeaseHistory::HeldRecovering(lease)
            | ScalarLeaseHistory::Released(lease) => {
                self.check_legacy_lease(&current, lease)?;
                Some(lease)
            }
            ScalarLeaseHistory::NoHistory => {
                if current.volume_id.is_some()
                    || current.lease_id.is_some()
                    || current.lease_fencing_token.is_some()
                {
                    return Err(invalid("Legacy original lease history is missing"));
                }
                None
            }
        };
        self.cleanup_legacy_transients(run).await?;
        if let Some(lease) = held {
            match &history {
                ScalarLeaseHistory::Held(_) => self.volumes.release_after_detach(lease).await?,
                ScalarLeaseHistory::HeldRecovering(_) => {
                    self.volumes.finish_recovery(lease).await?;
                }
                ScalarLeaseHistory::Released(_) | ScalarLeaseHistory::NoHistory => {}
            }
        }
        self.transition_legacy_cleanup(run, RunState::CleanedUp, None, None)
            .await?;
        self.legacy_terminal(run).await
    }
    pub(super) fn check_legacy_lease(
        &self,
        run: &Run,
        lease: &VolumeLease,
    ) -> Result<(), OrchestratorError> {
        let scope = self.legacy_owner()?;
        if lease.run_id != run.id
            || Some(lease.id) != run.lease_id
            || Some(lease.volume_id) != run.volume_id
            || Some(lease.fencing_token) != run.lease_fencing_token
            || lease.host_id != scope.provider_scope().host_id()
        {
            return Err(invalid(
                "Legacy global history differs from original tuple or host",
            ));
        }
        Ok(())
    }
    async fn transition_legacy_cleanup(
        &self,
        run: RunId,
        state: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, OrchestratorError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.repository.transition(run, state, exit, failure),
        )
        .await
        .map_err(|_| invalid("Legacy cleanup state recording deadline elapsed"))?
        .map_err(Into::into)
    }
    async fn read_legacy_run(&self, run: RunId) -> Result<Run, OrchestratorError> {
        tokio::time::timeout(std::time::Duration::from_secs(2), self.repository.get(run))
            .await
            .map_err(|_| invalid("Legacy cleanup Run read deadline elapsed"))?
            .map_err(Into::into)
    }
    async fn cleanup_legacy_transients(&self, run: RunId) -> Result<(), OrchestratorError> {
        tokio::time::timeout(std::time::Duration::from_secs(4), async {
            self.runtime_git_workspace.abandon_runtime_git(run).await?;
            self.authority.revoke_after_guest(run).await?;
            self.secrets.destroy_after_guest(run).await?;
            self.runtimes.destroy(run).await?;
            self.workspaces.abandon(run).await?;
            Ok::<(), OrchestratorError>(())
        })
        .await
        .map_err(|_| invalid("Legacy transient cleanup deadline elapsed"))?
    }
}
