use super::{
    LegacyVmPlacement, LegacyVmPlacementConsumption, LegacyVmPlacementError,
    LegacyVmPlacementProducer, LegacyVmPlacementScope, RunId, RunKind, StartRun, VmId,
    VmProviderOwnerScope,
};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId,
};

fn scope() -> LegacyVmPlacementScope {
    LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(RunId::new().to_string(), "host-1".into()).unwrap(),
    )
    .unwrap()
}

fn command() -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    }
}

#[test]
fn namespace_requires_actual_canonical_non_nil_owner_uuid() {
    for namespace in [
        "owner-kind",
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
    ] {
        assert_eq!(
            LegacyVmPlacementScope::new(
                VmProviderOwnerScope::new(namespace.into(), "host-1".into()).unwrap()
            ),
            Err(LegacyVmPlacementError::Namespace)
        );
    }
    let actual = scope();
    assert_eq!(actual.provider_scope().host_id(), "host-1");
}

#[test]
fn placement_does_not_substitute_another_runs_vm() {
    let input = command();
    assert_eq!(
        LegacyVmPlacement::new(
            input,
            uuid::Uuid::new_v4(),
            [1; 32],
            scope(),
            VmId(RunId::new().to_string()),
            LegacyVmPlacementProducer::NormalRequest,
            Some(LegacyVmPlacementConsumption::Provision)
        ),
        Err(LegacyVmPlacementError::VmIdentity)
    );
}

#[test]
fn producer_and_nil_pins_fail_closed() {
    let mut input = command();
    let vm = VmId(input.run_id.to_string());
    assert_eq!(
        LegacyVmPlacement::new(
            input.clone(),
            uuid::Uuid::new_v4(),
            [1; 32],
            scope(),
            vm.clone(),
            LegacyVmPlacementProducer::UpdateHook,
            None
        ),
        Err(LegacyVmPlacementError::Producer)
    );
    assert_eq!(
        LegacyVmPlacement::new(
            input.clone(),
            uuid::Uuid::nil(),
            [1; 32],
            scope(),
            vm.clone(),
            LegacyVmPlacementProducer::NormalRequest,
            None
        ),
        Err(LegacyVmPlacementError::Pins)
    );
    input.instance_id = "00000000-0000-0000-0000-000000000000".parse().unwrap();
    assert_eq!(
        LegacyVmPlacement::new(
            input,
            uuid::Uuid::new_v4(),
            [1; 32],
            scope(),
            vm,
            LegacyVmPlacementProducer::NormalRequest,
            None
        ),
        Err(LegacyVmPlacementError::Pins)
    );
}

#[test]
fn eligible_provision_and_cleanup_only_are_distinct_projections() {
    let mut input = command();
    input.kind = RunKind::Update;
    input.attachment_id = None;
    let vm = VmId(input.run_id.to_string());
    let owner = scope();
    let project = uuid::Uuid::new_v4();
    for consumption in [
        None,
        Some(LegacyVmPlacementConsumption::Provision),
        Some(LegacyVmPlacementConsumption::CleanupOnly),
    ] {
        let plan = LegacyVmPlacement::new(
            input.clone(),
            project,
            [7; 32],
            owner.clone(),
            vm.clone(),
            LegacyVmPlacementProducer::UpdateHook,
            consumption,
        )
        .unwrap();
        assert_eq!(plan.command(), &input);
        assert_eq!(plan.vm_id(), &vm);
        assert_eq!(plan.scope(), &owner);
        assert_eq!(plan.project(), project);
        assert_eq!(plan.contract_hash(), &[7; 32]);
        assert_eq!(plan.producer(), LegacyVmPlacementProducer::UpdateHook);
        assert_eq!(plan.consumption(), consumption);
    }
}
