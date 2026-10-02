use run_domain::{Run, RunState};
use runtime_types::RunId;
use std::sync::Arc;
use vm_trait::VmInstance;
use volume_trait::VolumeLease;

use super::{OrchestratorError, RunOrchestrator};

impl RunOrchestrator {
    pub(super) async fn fail_without_lease(
        &self,
        run_id: RunId,
        failure: &str,
    ) -> Result<Run, OrchestratorError> {
        if self.canonical_cleanup.is_some() {
            return self.fail_with_resources(run_id, None, None, failure).await;
        }
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

    pub(super) async fn fail_with_resources(
        &self,
        run_id: RunId,
        lease: Option<&VolumeLease>,
        instance: Option<Arc<dyn VmInstance>>,
        failure: &str,
    ) -> Result<Run, OrchestratorError> {
        self.repository
            .transition(run_id, RunState::Failed, None, Some(failure))
            .await?;
        let result = self.cleanup(run_id, lease, instance).await;
        if self.canonical_cleanup.is_some() {
            return result.map_err(|cleanup| OrchestratorError::CleanupIncomplete {
                failure: failure.to_owned(),
                cleanup: Box::new(cleanup),
            });
        }
        result
    }

    pub(super) async fn cancel_before_vm(
        &self,
        run: Run,
        lease: Option<&VolumeLease>,
    ) -> Result<Run, OrchestratorError> {
        self.repository
            .transition(run.id, RunState::Cancelled, None, None)
            .await?;
        self.cleanup(run.id, lease, None).await
    }

    pub(super) async fn cleanup(
        &self,
        run_id: RunId,
        lease: Option<&VolumeLease>,
        instance: Option<Arc<dyn VmInstance>>,
    ) -> Result<Run, OrchestratorError> {
        if self.canonical_cleanup.is_some() {
            return self
                .canonical_cleanup(run_id, instance)
                .await
                .map_err(|cleanup| OrchestratorError::CleanupIncomplete {
                    failure: String::from("run cleanup"),
                    cleanup: Box::new(cleanup),
                });
        }
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
        self.runtime_git_workspace
            .abandon_runtime_git(run_id)
            .await?;
        self.authority.revoke_after_guest(run_id).await?;
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

    pub(super) async fn abort_vm_keep_lease(&self, run_id: RunId, instance: &Arc<dyn VmInstance>) {
        if self.canonical_cleanup.is_some() {
            // Keep the exact live handle for canonical closure and scoped
            // confirmation. The claimed-run error path performs cleanup.
            self.active
                .lock()
                .await
                .insert(run_id, Arc::clone(instance));
            return;
        }
        if let Err(error) = instance.destroy().await {
            tracing::error!(%run_id, %error, "failed to destroy VM after orchestration error");
        }
        self.active.lock().await.remove(&run_id);
    }

    pub(super) async fn finish_recovered_run(&self, run: Run) -> Result<(), OrchestratorError> {
        if self.canonical_cleanup.is_some() {
            self.fail_claimed_run(
                run.id,
                "supervisor restarted while run resources were active",
            )
            .await?;
            return Ok(());
        }
        self.runtime_git_workspace
            .abandon_runtime_git(run.id)
            .await?;
        self.authority.revoke_after_guest(run.id).await?;
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
