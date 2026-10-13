//! Qualified original-family reconciliation; constructor/recovery is not Ready.
mod actions;
mod inventory;
#[cfg(test)]
mod tests;

use super::{ManagedRunCommandExecutor, held, intake::Operations};
use actions::Actions;
use run_domain::LegacyVmPlacementInventory;
use run_orchestrator::{OrchestratorError, SupervisorReconciliationCause};
use runtime_types::RunId;
use std::collections::BTreeSet;
use vm_trait::VmProviderOwnerScope;

// Private controlled seam; production is ONLY the original concrete executor.
#[async_trait::async_trait]
trait LifecycleActions: Sync {
    fn scope(&self) -> &VmProviderOwnerScope;
    fn validate(&self) -> Result<(), OrchestratorError>;
    async fn invocation_inventory(&self) -> Result<BTreeSet<RunId>, OrchestratorError>;
    async fn legacy_inventory(&self) -> Result<LegacyVmPlacementInventory, OrchestratorError>;
    async fn close_provider(&self) -> Result<(), OrchestratorError>;
    async fn drain_provider(&self) -> Result<(), OrchestratorError>;
    async fn reconcile_invocation(
        &self,
        cause: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError>;
    async fn reconcile_legacy(
        &self,
        cause: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError>;
    async fn native_barrier(&self) -> Result<(), OrchestratorError>;
}

pub(super) async fn run(
    executor: &ManagedRunCommandExecutor,
    operations: &Operations,
    cause: SupervisorReconciliationCause,
) -> Result<(), OrchestratorError> {
    reconcile(&Actions(executor), operations, cause).await
}

async fn preflight(actions: &impl LifecycleActions) -> Result<BTreeSet<RunId>, OrchestratorError> {
    // No family cleanup can precede qualification of the later family.
    let invocation = actions.invocation_inventory().await?;
    let legacy = actions.legacy_inventory().await?;
    inventory::validate(&invocation, &legacy, actions.scope())?;
    Ok(invocation)
}

async fn reconcile(
    actions: &impl LifecycleActions,
    operations: &Operations,
    cause: SupervisorReconciliationCause,
) -> Result<(), OrchestratorError> {
    actions.validate()?;
    if cause == SupervisorReconciliationCause::Stop {
        operations.close()?;
        actions.close_provider().await?;
    } else if !operations.idle_commands()? {
        return Err(held("managed restart overlaps an original owned command"));
    }
    if cause == SupervisorReconciliationCause::Restart {
        operations.wait_commands().await?;
    }
    preflight(actions).await?;
    if cause == SupervisorReconciliationCause::Stop {
        // Actual local worker termination precedes waiting the original Core
        // monitors/finalizers. No Cancel DTO or database receipt is invented.
        actions.drain_provider().await?;
        operations.wait_commands().await?;
    }
    actions.reconcile_invocation(cause).await?;
    actions.reconcile_legacy(cause).await?;
    if !preflight(actions).await?.is_empty() {
        return Err(held(
            "managed Invocation cleanup remains actionable after reconciliation",
        ));
    }
    actions.validate()?;
    actions.native_barrier().await?;
    if !operations.joined_commands()? {
        return Err(held(
            "managed cleanup was submitted during final observation",
        ));
    }
    // Historical caller errors remain available; complete qualified observations
    // alone decide this lifecycle outcome. Nothing retroactively ACKs a delivery.
    for cause in operations.failures()? {
        tracing::debug!(%cause, "managed original failure retained after qualified lifecycle observation");
    }
    Ok(())
}
