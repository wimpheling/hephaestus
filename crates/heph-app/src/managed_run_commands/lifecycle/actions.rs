//! Only this original executor can supply actual lifecycle effects.
use super::{
    LifecycleActions, ManagedRunCommandExecutor, OrchestratorError, SupervisorReconciliationCause,
};
use run_domain::LegacyVmPlacementInventory;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use std::collections::BTreeSet;
use vm_trait::VmProviderOwnerScope;

pub(super) struct Actions<'a>(pub(super) &'a ManagedRunCommandExecutor);
#[async_trait::async_trait]
impl LifecycleActions for Actions<'_> {
    fn scope(&self) -> &VmProviderOwnerScope {
        self.0
            .invocation
            .drain_inputs()
            .configuration
            .provider_scope()
    }
    fn validate(&self) -> Result<(), OrchestratorError> {
        self.0
            .invocation
            .validate_owners()
            .map_err(|cause| RepositoryError::Storage(Box::new(cause)))?;
        self.0
            .legacy
            .validate_owners()
            .map_err(|cause| RepositoryError::Storage(Box::new(cause)))?;
        Ok(())
    }
    async fn invocation_inventory(&self) -> Result<BTreeSet<RunId>, OrchestratorError> {
        self.0
            .invocation
            .orchestrator()
            .qualified_invocation_recovery_inventory()
            .await
    }
    async fn legacy_inventory(&self) -> Result<LegacyVmPlacementInventory, OrchestratorError> {
        self.0
            .legacy
            .placement_inventory()
            .await
            .map_err(Into::into)
    }
    async fn close_provider(&self) -> Result<(), OrchestratorError> {
        self.0
            .invocation
            .drain_inputs()
            .provider
            .close_admission(
                self.0
                    .invocation
                    .drain_inputs()
                    .configuration
                    .provider_scope(),
            )
            .await?;
        Ok(())
    }
    async fn drain_provider(&self) -> Result<(), OrchestratorError> {
        self.0
            .invocation
            .drain_inputs()
            .provider
            .drain_generation(
                self.0
                    .invocation
                    .drain_inputs()
                    .configuration
                    .provider_scope(),
            )
            .await?;
        Ok(())
    }
    async fn reconcile_invocation(
        &self,
        cause: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        self.0
            .invocation
            .orchestrator()
            .reconcile_managed_runs(cause)
            .await
    }
    async fn reconcile_legacy(
        &self,
        cause: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        self.0
            .legacy
            .managed_orchestrator()
            .reconcile_managed_runs(cause)
            .await
    }
    async fn native_barrier(&self) -> Result<(), OrchestratorError> {
        self.0
            .invocation
            .drain_inputs()
            .provider
            .assert_no_prior_births(
                self.0
                    .invocation
                    .drain_inputs()
                    .configuration
                    .provider_scope(),
            )
            .await?;
        self.0
            .invocation
            .drain_inputs()
            .volume_owner
            .validate()
            .map_err(|cause| RepositoryError::Storage(Box::new(cause)))?;
        self.validate()
    }
}
