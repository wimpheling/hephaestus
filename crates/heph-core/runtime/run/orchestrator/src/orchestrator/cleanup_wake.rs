//! Exact cancelled-Run cleanup; no new Cancel, global sweep or recipe outcome.
use super::{
    OrchestratorError, RunOrchestrator,
    canonical_cleanup::{host, invalid},
};
use crate::{
    RunCleanupWakeBinding, RunCleanupWakeProgress, RunCleanupWakeRequest, RunCleanupWakeSnapshot,
};
use run_domain::{LegacyVmPlacementConsumption, RunState};
use std::{sync::Arc, time::Duration};

impl RunOrchestrator {
    /// Wakes exact permanently cancelled cleanup using this same owning controller.
    ///
    /// Retains the actual active map/provider/guard family in a detached bounded
    /// task. Caller cancellation cannot abandon an awaited owning DB commit.
    /// Complete is progress only; later action-specific verification is required.
    ///
    /// # Errors
    /// Unsupported mode/qualification and contradictory owning evidence deny IO.
    pub async fn wake_cleanup(
        self: Arc<Self>,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        if self.legacy_scope.is_none() && self.canonical_cleanup.is_none() {
            return Err(invalid("owning cleanup wake is unsupported"));
        }
        let request = request.clone();
        let budget = self.cleanup_timeout.min(Duration::from_secs(30));
        tokio::spawn(async move {
            tokio::time::timeout(budget, self.wake_cleanup_inner(&request))
                .await
                .unwrap_or_else(|_| Ok(RunCleanupWakeProgress::Held))
        })
        .await
        .map_err(|_| invalid("owning cleanup wake completion is unknown"))?
    }

    async fn wake_cleanup_inner(
        &self,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        let scope = host(&self.provider.owner_scope()?)?;
        let start = match self.operation_guards.claim_start(request.run_id()) {
            Ok(start) => start,
            Err(OrchestratorError::RunInProgress(_)) => return Ok(RunCleanupWakeProgress::Held),
            Err(error) => return Err(error),
        };
        let operation = self.lock_run_operation(request.run_id()).await?;
        let initial = self
            .repository
            .prepare_cleanup_wake(request, &scope)
            .await?;
        self.check_cleanup_wake(request, &initial)?;
        let fresh = self.repository.load_cleanup_wake(request, &scope).await?;
        self.check_cleanup_wake(request, &fresh)?;
        if fresh != initial {
            return Ok(RunCleanupWakeProgress::Held);
        }
        // Existing cleanup reacquires the operation mutex. Permanent backend
        // closure and this retained live-Start claim exclude new IO in the gap.
        drop(operation);
        if matches!(fresh.binding, RunCleanupWakeBinding::Owned { .. })
            && fresh.run.state == RunState::CleanedUp
            && self
                .canonical_cleanup
                .as_ref()
                .ok_or_else(|| invalid("canonical cleanup is absent"))?
                .repository
                .recorded_cleanup_receipt(request.run_id())
                .await?
                .is_none()
        {
            return Ok(RunCleanupWakeProgress::Held);
        }
        let run = self.cleanup_qualified_wake(request, &fresh).await?;
        // Do not accept a helper returning another Run or a bare earlier status.
        if run.id != request.run_id()
            || run.instance_id != request.instance_id()
            || run.state != RunState::CleanedUp
        {
            return Err(invalid(
                "cleanup wake returned contradictory terminal progress",
            ));
        }
        let completed = self.repository.load_cleanup_wake(request, &scope).await?;
        self.check_cleanup_wake(request, &completed)?;
        if completed.run != run
            || completed.binding != fresh.binding
            || !immutable_run_matches(&fresh.run, &completed.run)
        {
            return Ok(RunCleanupWakeProgress::Held);
        }
        drop(start);
        Ok(RunCleanupWakeProgress::Complete)
    }

    async fn cleanup_qualified_wake(
        &self,
        request: &RunCleanupWakeRequest,
        fresh: &RunCleanupWakeSnapshot,
    ) -> Result<run_domain::Run, OrchestratorError> {
        match &fresh.binding {
            RunCleanupWakeBinding::Legacy(plan) => {
                let previous = {
                    self.legacy_plans
                        .lock()
                        .await
                        .get(&request.run_id())
                        .cloned()
                };
                if let Some(previous) = previous {
                    self.check_legacy_plan_return(request.run_id(), &previous, plan)?;
                    if previous.consumption() != plan.consumption() {
                        return Err(invalid("cleanup wake owning replay is not qualified"));
                    }
                }
                self.legacy_plans
                    .lock()
                    .await
                    .insert(request.run_id(), (**plan).clone());
                self.cleanup_legacy(
                    request.run_id(),
                    None,
                    Some(("permanent instance removal cancellation", None)),
                )
                .await
            }
            RunCleanupWakeBinding::Owned { target } => {
                if !matches!(
                    fresh.run.state,
                    RunState::Succeeded
                        | RunState::Failed
                        | RunState::Cancelled
                        | RunState::CleaningUp
                        | RunState::CleanedUp
                ) {
                    self.repository
                        .transition(request.run_id(), RunState::Cancelled, None, None)
                        .await?;
                }
                self.canonical_cleanup_expected(request.run_id(), None, Some(target))
                    .await
            }
        }
    }

    fn check_cleanup_wake(
        &self,
        request: &RunCleanupWakeRequest,
        snapshot: &RunCleanupWakeSnapshot,
    ) -> Result<(), OrchestratorError> {
        if &snapshot.request != request
            || snapshot.run.id != request.run_id()
            || snapshot.run.instance_id != request.instance_id()
            || snapshot.run.cancel_requested_at.is_none()
        {
            return Err(invalid(
                "cleanup wake does not match original closed cancellation",
            ));
        }
        match &snapshot.binding {
            RunCleanupWakeBinding::Legacy(plan) => {
                self.check_legacy_run_pins(&snapshot.run, plan)?;
                if plan.consumption() != Some(LegacyVmPlacementConsumption::Provision)
                    && plan.consumption() != Some(LegacyVmPlacementConsumption::CleanupOnly)
                {
                    return Err(invalid("cleanup wake Legacy placement is not consumed"));
                }
            }
            RunCleanupWakeBinding::Owned { target } => {
                if self.legacy_scope.is_some()
                    || self.canonical_cleanup.is_none()
                    || target.run_id() != request.run_id()
                    || target.instance_id() != snapshot.run.instance_id
                    || target.revision_id() != snapshot.run.instance_revision_id
                    || snapshot.run.vm_id.as_deref()
                        != target.vm_target().vm_id().map(|id| id.0.as_str())
                {
                    return Err(invalid("cleanup wake canonical target differs"));
                }
                self.validate_cleanup_scope(target)?;
            }
        }
        Ok(())
    }
}

fn immutable_run_matches(before: &run_domain::Run, after: &run_domain::Run) -> bool {
    before.id == after.id
        && before.instance_id == after.instance_id
        && before.instance_revision_id == after.instance_revision_id
        && before.release_id == after.release_id
        && before.release_agent_id == after.release_agent_id
        && before.attachment_id == after.attachment_id
        && before.kind == after.kind
        && before.requires_state == after.requires_state
        && before.command_id == after.command_id
        && before.volume_id == after.volume_id
        && before.lease_id == after.lease_id
        && before.lease_fencing_token == after.lease_fencing_token
        && before.vm_id == after.vm_id
        && before.created_at == after.created_at
        && before.cancel_requested_at == after.cancel_requested_at
        && (!matches!(
            before.state,
            RunState::Succeeded
                | RunState::Failed
                | RunState::Cancelled
                | RunState::CleaningUp
                | RunState::CleanedUp
        ) || (before.outcome == after.outcome && before.exit == after.exit))
}
