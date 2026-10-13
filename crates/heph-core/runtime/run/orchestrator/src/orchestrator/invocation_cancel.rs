//! Exact queued cancellation preparation before strict hydration and physical cleanup.
use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};
use crate::{
    InvocationCancelEvidence, InvocationCancelPreparation, PreparedInvocationCancellation,
};
use run_domain::{CancelRun, RunKind, RunOutcome, RunState};
use runtime_types::RunId;
use vm_trait::{StopMode, VmProviderOwnerScope};

impl RunOrchestrator {
    pub(super) fn invocation_cancel_owner(
        &self,
    ) -> Result<VmProviderOwnerScope, OrchestratorError> {
        let scope = self.provider.owner_scope()?;
        if self.invocation_scope.as_ref() != Some(&scope)
            || self.legacy_scope.is_some()
            || self.canonical_cleanup.is_none()
        {
            return Err(invalid("Invocation cancellation owner/composition differs"));
        }
        Ok(scope)
    }

    pub(super) async fn prepare_invocation_cancel(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<Option<PreparedInvocationCancellation>, OrchestratorError> {
        let prepared = match self
            .repository
            .prepare_invocation_cancel_cleanup(run, scope)
            .await?
        {
            InvocationCancelPreparation::Consumed117 => return Ok(None),
            InvocationCancelPreparation::PreparedQueued(prepared) => *prepared,
        };
        self.check_prepared_invocation_cancel(run, &prepared)?;
        // Strict loading follows protected preparation. A captured projection
        // cannot hide intervening changes, substitute pins or permit adoption.
        if self.repository.get(run).await? != prepared.run {
            return Err(invalid("prepared Invocation cancellation Run changed"));
        }
        Ok(Some(prepared))
    }

    fn check_prepared_invocation_cancel(
        &self,
        run_id: RunId,
        prepared: &PreparedInvocationCancellation,
    ) -> Result<(), OrchestratorError> {
        let run = &prepared.run;
        let target = &prepared.target;
        if run.id != run_id
            || run.kind != RunKind::Invocation
            || run.attachment_id.is_some()
            || run.vm_id.as_deref() != Some(run_id.to_string().as_str())
            || run.cancel_requested_at.is_none()
            || !matches!(
                run.state,
                RunState::Queued | RunState::Cancelled | RunState::CleaningUp | RunState::CleanedUp
            )
            || (run.state == RunState::Queued && run.outcome.is_some())
            || (run.state != RunState::Queued && run.outcome != Some(RunOutcome::Cancelled))
            || run.exit.is_some()
            || run.failure.is_some()
            || run.volume_id.is_some()
            || run.lease_id.is_some()
            || run.lease_fencing_token.is_some()
            || target.run_id() != run_id
            || target.instance_id() != run.instance_id
            || target.revision_id() != run.instance_revision_id
            || !target.leases().is_empty()
            || target.vm_target().vm_id().map(|id| id.0.as_str()) != run.vm_id.as_deref()
        {
            return Err(invalid(
                "prepared queued Invocation cancellation pins differ",
            ));
        }
        self.validate_cleanup_scope(target)?;
        Ok(())
    }

    pub(super) async fn cancel_invocation_run(
        &self,
        command: &CancelRun,
        inserted: bool,
    ) -> Result<bool, OrchestratorError> {
        let scope = self.invocation_cancel_owner()?;
        if self
            .repository
            .classify_invocation_cancel(command.run_id, &scope)
            .await?
            == InvocationCancelEvidence::Consumed117
        {
            self.stop_registered_invocation(command.run_id).await?;
            return Ok(inserted);
        }
        let start = match self.operation_guards.claim_start(command.run_id) {
            Ok(start) => start,
            Err(OrchestratorError::RunInProgress(run)) => {
                // Do not queue Cancel behind live Start or steal its admission.
                self.stop_registered_invocation(run).await?;
                return Err(OrchestratorError::RunInProgress(run));
            }
            Err(error) => return Err(error),
        };
        let operation = self.lock_run_operation(command.run_id).await?;
        let prepared = self
            .prepare_invocation_cancel(command.run_id, &scope)
            .await?;
        let Some(prepared) = prepared else {
            drop(operation);
            self.stop_registered_invocation(command.run_id).await?;
            return Err(OrchestratorError::RunInProgress(command.run_id));
        };
        if prepared.run.state == RunState::Queued {
            self.repository
                .transition(command.run_id, RunState::Cancelled, None, None)
                .await?;
        }
        // Acquisition is permanently closed; retain admission while actual
        // canonical cleanup reacquires this SAME operation mutex.
        drop(operation);
        self.canonical_cleanup_expected(command.run_id, None, Some(&prepared.target))
            .await?;
        drop(start);
        Ok(inserted)
    }

    async fn stop_registered_invocation(&self, run: RunId) -> Result<(), OrchestratorError> {
        let instance = self.active.lock().await.get(&run).cloned();
        if let Some(instance) = instance {
            if instance.id().0 != run.to_string() {
                return Err(invalid(
                    "registered Invocation VM differs from original Run",
                ));
            }
            instance
                .stop(StopMode::Graceful {
                    timeout: self.cancellation_timeout,
                })
                .await?;
        }
        Ok(())
    }
}
