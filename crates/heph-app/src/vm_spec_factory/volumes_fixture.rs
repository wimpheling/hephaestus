use capability_domain::{AuthorityHash, CapabilitySlotKey};
use control_plane_postgres::run::VmLaunchContract;
use heph_run::{Run, RunKind, RunState};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_domain::{
    GuestMountPath, RunVolumeIdentity, RunVolumeSelection, RunVolumeSelections, VolumeAccessMode,
    VolumeMountScope, VolumeSelectionOrigin, VolumeSlotDeclaration,
};

pub fn fixture() -> (Run, VmLaunchContract) {
    let run = Run {
        id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Normal,
        requires_state: false,
        command_id: CommandId::new(),
        volume_id: None,
        lease_id: None,
        lease_fencing_token: None,
        vm_id: None,
        state: RunState::Provisioning,
        outcome: None,
        exit: None,
        failure: None,
        cancel_requested_at: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
        state_version: 1,
    };
    let stored = VmLaunchContract {
        run_id: run.id.as_uuid(),
        instance_id: run.instance_id.as_uuid(),
        instance_revision_id: run.instance_revision_id.as_uuid(),
        release_id: run.release_id.as_uuid(),
        release_agent_id: run.release_agent_id.as_uuid(),
        project_id: Uuid::new_v4(),
        runtime_contract_hash: vec![7; 32],
        volume_mode: "named".into(),
        state_volume_id: None,
        runtime_contract: serde_json::json!({
            "command":"app.py", "arguments":["inspect"], "working_directory":".",
            "image_reference":"test-image", "requires_state":false
        }),
        effective_runtime_policy: serde_json::json!({"vcpus":1,"memory_mib":512,"network":"disabled"}),
        requires_state: false,
        update_hook: None,
        release_state: "published".into(),
        revision_runnable: true,
        attachment_runnable: true,
        agent_update_id: None,
    };
    (run, stored)
}

pub fn identity(stored: &VmLaunchContract) -> RunVolumeIdentity {
    RunVolumeIdentity::new(
        RunId::from_uuid(stored.run_id),
        AgentInstanceId::from_uuid(stored.instance_id),
        AgentInstanceRevisionId::from_uuid(stored.instance_revision_id),
        ReleaseId::from_uuid(stored.release_id),
        ReleaseAgentId::from_uuid(stored.release_agent_id),
        stored.project_id,
    )
    .unwrap()
}

pub fn declaration(required: bool) -> VolumeSlotDeclaration {
    VolumeSlotDeclaration::new(
        CapabilitySlotKey::parse("data").unwrap(),
        GuestMountPath::parse("/data").unwrap(),
        VolumeAccessMode::ReadWrite,
        required,
        16_777_216,
    )
    .unwrap()
}

pub fn selection(
    stored: &VmLaunchContract,
    declaration: VolumeSlotDeclaration,
    origin: VolumeSelectionOrigin,
) -> RunVolumeSelection {
    let scope = VolumeMountScope::new(
        AgentInstanceId::from_uuid(stored.instance_id),
        AgentInstanceRevisionId::from_uuid(stored.instance_revision_id),
        ReleaseAgentId::from_uuid(stored.release_agent_id),
        declaration.slot().clone(),
        VolumeId::from_uuid(stored.state_volume_id.unwrap_or_else(Uuid::new_v4)),
        declaration.access_mode(),
        AuthorityHash::from_bytes(stored.runtime_contract_hash.as_slice().try_into().unwrap()),
    )
    .unwrap();
    RunVolumeSelection::new(identity(stored), scope, declaration, origin).unwrap()
}

pub fn set(stored: &VmLaunchContract, selected: Vec<RunVolumeSelection>) -> RunVolumeSelections {
    RunVolumeSelections::new(identity(stored), selected).unwrap()
}

pub fn catalog(stored: &mut VmLaunchContract, declarations: &[VolumeSlotDeclaration]) {
    stored.runtime_contract["volume_slots"] = serde_json::to_value(declarations).unwrap();
}
