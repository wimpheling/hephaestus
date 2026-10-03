//! Opt-in durable cleanup; local attachment values never prove the resource set.

use run_domain::{Run, RunCleanupHostId, RunState, StartRun};
use runtime_types::RunId;
use std::sync::Arc;
use vm_trait::{VmId, VmInstance, VmProviderOwnerScope};
use volume_trait::RunVolumeStore;

use super::{OrchestratorError, RunOrchestrator};
use crate::{RepositoryError, RunCleanupRepository};

pub struct CanonicalCleanup {
    pub repository: Arc<dyn RunCleanupRepository>,
    pub volumes: Arc<dyn RunVolumeStore>,
}

pub fn invalid(message: &'static str) -> OrchestratorError {
    RepositoryError::InvalidData(message).into()
}

pub fn checked<T>(result: Result<T, run_domain::RunCleanupError>) -> Result<T, OrchestratorError> {
    result.map_err(|error| RepositoryError::Storage(Box::new(error)).into())
}

pub fn host(scope: &VmProviderOwnerScope) -> Result<RunCleanupHostId, OrchestratorError> {
    checked(RunCleanupHostId::new(
        scope.namespace().to_owned(),
        scope.host_id().to_owned(),
    ))
}

impl RunOrchestrator {
    pub(super) async fn create_planned_run(
        &self,
        command: &StartRun,
    ) -> Result<crate::CreateRunResult, OrchestratorError> {
        let scope = self.provider.owner_scope()?;
        let vm_id = VmId(command.run_id.to_string());
        self.repository
            .create_run_with_vm_plan(command, &host(&scope)?, &vm_id)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn bind_planned_vm(&self, run: &Run) -> Result<(), OrchestratorError> {
        if self.legacy_scope.is_some() {
            self.check_legacy_open(run).await?;
            return Ok(());
        }
        let Some(cleanup) = self.canonical_cleanup.as_ref() else {
            return Ok(());
        };
        self.operation_guards.check_io(run.id)?;
        let scope = self.provider.owner_scope()?;
        let vm_id = match run.vm_id.as_ref() {
            Some(id) => VmId(id.clone()),
            None => return Err(invalid("historical run has no proven planned VM identity")),
        };
        cleanup
            .repository
            .bind_vm_before_provision(
                run.id,
                run.instance_id,
                run.instance_revision_id,
                &host(&scope)?,
                &vm_id,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn canonical_cleanup(
        &self,
        run_id: RunId,
        instance: Option<Arc<dyn VmInstance>>,
    ) -> Result<Run, OrchestratorError> {
        let cleanup = self
            .canonical_cleanup
            .as_ref()
            .ok_or_else(|| invalid("canonical cleanup is not configured"))?;
        // Check the actual provider even for receipt replay and zero leases.
        self.provider.owner_scope()?;
        let run = self.repository.get(run_id).await?;
        if run.state == RunState::CleanedUp {
            return self.replay_cleaned_run(run, cleanup).await;
        }
        let receipt = self
            .confirm_canonical_guest_cleanup(run_id, instance)
            .await?;
        let current = self.repository.get(run_id).await?;
        if current.state != RunState::CleaningUp {
            self.repository
                .transition(run_id, RunState::CleaningUp, None, None)
                .await?;
        }
        self.cleanup_transient_resources(run_id).await?;
        let cleaned = cleanup.repository.finish_cleanup(&receipt).await?;
        self.completion.after_cleanup(&cleaned).await?;
        Ok(cleaned)
    }

    async fn replay_cleaned_run(
        &self,
        run: Run,
        cleanup: &CanonicalCleanup,
    ) -> Result<Run, OrchestratorError> {
        let cleaned =
            if let Some(receipt) = cleanup.repository.recorded_cleanup_receipt(run.id).await? {
                self.validate_cleanup_scope(receipt.target())?;
                // A completed target can reference resources since reused by a new
                // run. Its exact completion replay must precede any provider IO.
                cleanup.repository.finish_cleanup(&receipt).await?
            } else {
                if !cleanup.volumes.leases_for_run(run.id).await?.is_empty() {
                    return Err(invalid(
                        "cleaned historical run still holds canonical leases",
                    ));
                }
                // This grandfathering applies only to this persisted terminal run.
                // It creates no receipt or evidence for any other run or resource.
                run
            };
        self.completion.after_cleanup(&cleaned).await?;
        Ok(cleaned)
    }

    async fn cleanup_transient_resources(&self, run_id: RunId) -> Result<(), OrchestratorError> {
        self.runtime_git_workspace
            .abandon_runtime_git(run_id)
            .await?;
        self.authority.revoke_after_guest(run_id).await?;
        self.secrets.destroy_after_guest(run_id).await?;
        self.runtimes.destroy(run_id).await?;
        self.workspaces.abandon(run_id).await?;
        Ok(())
    }

    pub(super) async fn fail_claimed_run(
        &self,
        run_id: RunId,
        failure: &str,
    ) -> Result<Run, OrchestratorError> {
        let run = self.repository.get(run_id).await?;
        if !matches!(
            run.state,
            RunState::Succeeded
                | RunState::Failed
                | RunState::Cancelled
                | RunState::CleaningUp
                | RunState::CleanedUp
        ) {
            self.repository
                .transition(run_id, RunState::Failed, None, Some(failure))
                .await?;
        }
        self.canonical_cleanup(run_id, None).await
    }
}
