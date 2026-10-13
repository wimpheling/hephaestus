//! Complete already-qualified snapshots; no profile or physical proof inference.
use super::{
    BTreeSet, LegacyVmPlacementInventory, OrchestratorError, RunId, VmProviderOwnerScope, held,
};

pub(super) fn validate(
    invocation: &BTreeSet<RunId>,
    legacy: &LegacyVmPlacementInventory,
    scope: &VmProviderOwnerScope,
) -> Result<(), OrchestratorError> {
    if !legacy.held.is_empty() || invocation.iter().any(|run| run.as_uuid().is_nil()) {
        return Err(held(
            "managed family inventory contains Held or invalid originals",
        ));
    }
    let mut placed = BTreeSet::new();
    for plan in &legacy.placements {
        let run = plan.command().run_id;
        if plan.scope().provider_scope() != scope
            || !placed.insert(run)
            || invocation.contains(&run)
        {
            return Err(held(
                "managed family inventories contain foreign, duplicate or conflicting originals",
            ));
        }
    }
    let mut closed = BTreeSet::new();
    for run in &legacy.closed_runs {
        if !placed.contains(run) || !closed.insert(*run) {
            return Err(held(
                "managed Legacy closed inventory has duplicate or orphan associations",
            ));
        }
    }
    Ok(())
}
