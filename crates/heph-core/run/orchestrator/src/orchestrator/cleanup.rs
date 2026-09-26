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
        self.cleanup(run_id, lease, instance).await
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
        if let Err(error) = instance.destroy().await {
            tracing::error!(%run_id, %error, "failed to destroy VM after orchestration error");
        }
        self.active.lock().await.remove(&run_id);
    }

    pub(super) async fn finish_recovered_run(&self, run: Run) -> Result<(), OrchestratorError> {
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
