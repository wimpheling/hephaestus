//! Lossless already-qualified projection shape; no native admission is minted.
use super::{BTreeSet, LegacyVmPlacementInventory, RunId, VmProviderOwnerScope, inventory};
use run_domain::{
    LegacyVmPlacement, LegacyVmPlacementProducer, LegacyVmPlacementScope, RunKind, StartRun,
};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId, ReleaseId,
};
use vm_trait::VmId;

fn placement(scope: &VmProviderOwnerScope) -> LegacyVmPlacement {
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: true,
    };
    let id = VmId(command.run_id.to_string());
    LegacyVmPlacement::new(
        command,
        RunId::new().as_uuid(),
        [1; 32],
        LegacyVmPlacementScope::new(scope.clone()).unwrap(),
        id,
        LegacyVmPlacementProducer::UpdateHook,
        None,
    )
    .unwrap()
}

#[test]
fn foreign_duplicate_orphan_and_cross_family_conflicts_deny_complete_snapshot() {
    let scope = VmProviderOwnerScope::new(RunId::new().to_string(), "host-1".into()).unwrap();
    let plan = placement(&scope);
    let mut legacy = LegacyVmPlacementInventory {
        placements: vec![plan.clone()],
        closed_runs: vec![],
        held: vec![],
    };
    assert!(inventory::validate(&BTreeSet::new(), &legacy, &scope).is_ok());
    legacy.placements.push(plan.clone());
    assert!(inventory::validate(&BTreeSet::new(), &legacy, &scope).is_err());
    legacy.placements.pop();
    legacy.closed_runs.push(RunId::new());
    assert!(inventory::validate(&BTreeSet::new(), &legacy, &scope).is_err());
    legacy.closed_runs.clear();
    assert!(
        inventory::validate(&BTreeSet::from([plan.command().run_id]), &legacy, &scope).is_err()
    );
    let foreign = VmProviderOwnerScope::new(RunId::new().to_string(), "host-1".into()).unwrap();
    assert!(inventory::validate(&BTreeSet::new(), &legacy, &foreign).is_err());
    legacy.closed_runs.push(plan.command().run_id);
    legacy.closed_runs.push(plan.command().run_id);
    assert!(inventory::validate(&BTreeSet::new(), &legacy, &scope).is_err());
}
