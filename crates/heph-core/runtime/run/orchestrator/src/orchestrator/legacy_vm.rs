//! Scoped physical cleanup while retaining every durable fence on uncertainty.
use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};
use run_domain::LegacyVmPlacement;
use std::sync::Arc;
use vm_trait::VmInstance;

impl RunOrchestrator {
    pub(super) async fn stop_legacy_guest(
        &self,
        run: runtime_types::RunId,
        instance: &Arc<dyn VmInstance>,
    ) -> Result<(), OrchestratorError> {
        tokio::time::timeout(self.cleanup_timeout, async {
            let operation = self.lock_run_operation(run).await?;
            let cached = self
                .legacy_plans
                .lock()
                .await
                .get(&run)
                .cloned()
                .ok_or_else(|| invalid("Legacy guest lacks checked placement"))?;
            if self.legacy_owner()? != cached.scope() || instance.id() != cached.vm_id() {
                return Err(invalid("Legacy stop identity differs"));
            }
            instance.stop(vm_trait::StopMode::Force).await?;
            drop(operation);
            Ok(())
        })
        .await
        .map_err(|_| invalid("Legacy stop deadline elapsed"))?
    }
    pub(super) async fn confirm_legacy_vm(
        &self,
        plan: &LegacyVmPlacement,
        supplied: Option<Arc<dyn VmInstance>>,
    ) -> Result<(), OrchestratorError> {
        let run = plan.command().run_id;
        let operation = self.lock_run_operation(run).await?;
        if self.legacy_owner()? != plan.scope() {
            return Err(invalid("Legacy cleanup scope differs"));
        }
        let instance = match supplied {
            Some(instance) => Some(instance),
            None => self.active.lock().await.get(&run).cloned(),
        };
        if let Some(instance) = &instance {
            if instance.id() != plan.vm_id() {
                return Err(invalid("Legacy live handle differs from recorded plan"));
            }
            instance.destroy().await?;
        }
        self.provider
            .cleanup_orphan_scoped(plan.scope().provider_scope(), plan.vm_id())
            .await?;
        self.legacy_owner()?;
        self.active.lock().await.remove(&run);
        drop(operation);
        Ok(())
    }
    pub(super) async fn close_confirm_legacy(
        &self,
        run: runtime_types::RunId,
        supplied: Option<Arc<dyn VmInstance>>,
    ) -> Result<LegacyVmPlacement, OrchestratorError> {
        self.close_confirm_legacy_until(
            run,
            supplied,
            tokio::time::Instant::now() + self.cleanup_timeout,
        )
        .await
    }
    pub(super) async fn close_confirm_legacy_until(
        &self,
        run: runtime_types::RunId,
        supplied: Option<Arc<dyn VmInstance>>,
        deadline: tokio::time::Instant,
    ) -> Result<LegacyVmPlacement, OrchestratorError> {
        let scope = self.legacy_owner()?.clone();
        // Start validation or checked recovery inventory supplies this identity.
        // A returned close DTO cannot select another Run's operation/handle.
        let expected = self
            .legacy_plans
            .lock()
            .await
            .get(&run)
            .cloned()
            .ok_or_else(|| invalid("Legacy cleanup lacks previously checked placement"))?;
        self.check_legacy_plan_return(run, &expected, &expected)?;
        let close = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.repository.close_legacy_vm_acquisition(run, &scope),
        )
        .await;
        let mut plan = match close {
            Ok(Ok(plan)) => plan,
            failure => {
                self.operation_guards.quarantine(run)?;
                tokio::time::timeout_at(deadline, self.confirm_legacy_vm(&expected, supplied))
                    .await
                    .map_err(|_| invalid("Legacy physical fallback deadline elapsed"))??;
                return Err(match failure {
                    Ok(Err(error)) => error.into(),
                    _ => invalid("Legacy closure deadline elapsed"),
                });
            }
        };
        if let Err(error) = self.check_legacy_plan_return(run, &expected, &plan) {
            self.operation_guards.quarantine(run)?;
            return Err(error);
        }
        if plan.consumption() != expected.consumption() {
            return Err(invalid("Legacy close changed placement consumption"));
        }
        if plan.consumption().is_none() {
            plan = tokio::time::timeout_at(
                deadline,
                self.repository.consume_legacy_vm_placement(
                    plan.command(),
                    &scope,
                    run_domain::LegacyVmPlacementConsumption::CleanupOnly,
                ),
            )
            .await
            .map_err(|_| invalid("Legacy cleanup consumption deadline elapsed"))??;
            if let Err(error) = self.check_legacy_plan_return(run, &expected, &plan) {
                self.operation_guards.quarantine(run)?;
                return Err(error);
            }
            if plan.consumption() != Some(run_domain::LegacyVmPlacementConsumption::CleanupOnly) {
                return Err(invalid(
                    "Legacy cleanup consumption returned another purpose",
                ));
            }
            self.legacy_plans.lock().await.insert(run, plan.clone());
        }
        tokio::time::timeout_at(deadline, self.confirm_legacy_vm(&plan, supplied))
            .await
            .map_err(|_| invalid("Legacy scoped cleanup deadline elapsed"))??;
        Ok(plan)
    }
}
