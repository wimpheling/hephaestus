//! Positive placement admission; no `RunUUID` fallback or physical absence claim.
use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};
use run_domain::{LegacyVmPlacement, LegacyVmPlacementConsumption, Run, RunState, StartRun};
use runtime_types::RunId;

impl RunOrchestrator {
    pub(super) fn legacy_owner(
        &self,
    ) -> Result<&run_domain::LegacyVmPlacementScope, OrchestratorError> {
        if self.canonical_cleanup.is_some() || self.plural_volumes {
            return Err(invalid("strict Legacy and canonical modes are exclusive"));
        }
        let scope = self
            .legacy_scope
            .as_ref()
            .ok_or_else(|| invalid("strict Legacy scope is absent"))?;
        if self.provider.owner_scope()? != *scope.provider_scope() {
            return Err(invalid("managed Legacy provider owner differs"));
        }
        Ok(scope)
    }
    pub(super) async fn create_legacy_run(
        &self,
        command: &StartRun,
    ) -> Result<crate::CreateRunResult, OrchestratorError> {
        let scope = self.legacy_owner()?;
        self.operation_guards.check_io(command.run_id)?;
        let created = self
            .repository
            .create_run_with_legacy_placement(command, scope)
            .await?;
        if created.run.state != RunState::CleanedUp {
            self.check_legacy_open(&created.run).await?;
        }
        Ok(created)
    }
    pub(super) async fn check_legacy_open(
        &self,
        run: &Run,
    ) -> Result<LegacyVmPlacement, OrchestratorError> {
        self.operation_guards.check_io(run.id)?;
        let scope = self.legacy_owner()?;
        let plan = self
            .repository
            .assert_legacy_vm_placement_open(run.id, scope)
            .await?;
        self.check_legacy_run_pins(run, &plan)?;
        if plan.consumption() != Some(LegacyVmPlacementConsumption::Provision) {
            return Err(invalid("Legacy placement is not consumed for provisioning"));
        }
        self.legacy_plans.lock().await.insert(run.id, plan.clone());
        Ok(plan)
    }
    pub(super) fn check_legacy_run_pins(
        &self,
        run: &Run,
        plan: &LegacyVmPlacement,
    ) -> Result<(), OrchestratorError> {
        if plan.scope() != self.legacy_owner()? {
            return Err(invalid(
                "Legacy placement scope differs from configured owner",
            ));
        }
        if plan.command().run_id != run.id
            || plan.command().instance_id != run.instance_id
            || plan.command().instance_revision_id != run.instance_revision_id
            || plan.command().release_id != run.release_id
            || plan.command().release_agent_id != run.release_agent_id
            || plan.command().command_id != run.command_id
            || plan.command().kind != run.kind
            || plan.command().requires_state != run.requires_state
            || plan.command().attachment_id != run.attachment_id
        {
            return Err(invalid("Legacy Run pins differ from positive placement"));
        }
        if run
            .vm_id
            .as_deref()
            .is_some_and(|id| id != plan.vm_id().0.as_str())
        {
            return Err(invalid(
                "Legacy placement differs from persisted VM identity",
            ));
        }
        Ok(())
    }
    pub(super) fn check_legacy_plan_return(
        &self,
        run: RunId,
        expected: &LegacyVmPlacement,
        returned: &LegacyVmPlacement,
    ) -> Result<(), OrchestratorError> {
        if expected.command().run_id != run
            || returned.command().run_id != run
            || returned.scope() != self.legacy_owner()?
            || returned.command() != expected.command()
            || returned.project() != expected.project()
            || returned.contract_hash() != expected.contract_hash()
            || returned.scope() != expected.scope()
            || returned.vm_id() != expected.vm_id()
            || returned.producer() != expected.producer()
        {
            return Err(invalid(
                "Legacy adapter returned a different immutable placement",
            ));
        }
        Ok(())
    }
    pub(super) async fn finish_legacy_start_result(
        &self,
        run: RunId,
        result: Result<Run, OrchestratorError>,
    ) -> Result<Run, OrchestratorError> {
        match result {
            Err(
                error @ (OrchestratorError::CleanupIncomplete { .. }
                | OrchestratorError::RunInProgress(_)),
            ) => Err(error),
            Err(error) => {
                let message = error.to_string();
                if let Err(cleanup) = self.cleanup_legacy(run, None, Some((&message, None))).await {
                    return Err(OrchestratorError::CleanupIncomplete {
                        failure: message,
                        cleanup: Box::new(cleanup),
                    });
                }
                Err(error)
            }
            Ok(run) => Ok(run),
        }
    }
    pub(super) async fn legacy_terminal(&self, run: RunId) -> Result<Run, OrchestratorError> {
        let completed = self
            .repository
            .legacy_vm_cleanup_completed(run, self.legacy_owner()?)
            .await?;
        self.operation_guards.clear_quarantine(run)?;
        self.completion.after_cleanup(&completed).await?;
        self.legacy_plans.lock().await.remove(&run);
        Ok(completed)
    }
}
