use capability_domain::{AuthorityHash, CapabilitySlotKey};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use uuid::Uuid;

use super::*;
use crate::GuestMountPath;

fn fixture_identity() -> RunVolumeIdentity {
    RunVolumeIdentity::new(
        RunId::new(),
        AgentInstanceId::new(),
        AgentInstanceRevisionId::new(),
        ReleaseId::new(),
        ReleaseAgentId::new(),
        Uuid::new_v4(),
    )
    .unwrap()
}

fn selection(
    identity: RunVolumeIdentity,
    name: &str,
    path: &str,
    volume: VolumeId,
    mode: VolumeAccessMode,
    origin: VolumeSelectionOrigin,
) -> RunVolumeSelection {
    let slot = CapabilitySlotKey::parse(name).unwrap();
    let scope = VolumeMountScope::new(
        identity.instance_id(),
        identity.revision_id(),
        identity.release_agent_id(),
        slot.clone(),
        volume,
        mode,
        AuthorityHash::from_bytes([7; 32]),
    )
    .unwrap();
    let declaration =
        VolumeSlotDeclaration::new(slot, GuestMountPath::parse(path).unwrap(), mode, true, 1)
            .unwrap();
    RunVolumeSelection::new(identity, scope, declaration, origin).unwrap()
}

#[test]
fn empty_and_plural_sets_preserve_exact_identity_and_canonical_order() {
    let identity = fixture_identity();
    let empty = RunVolumeSelections::new(identity, vec![]).unwrap();
    assert_eq!(empty.identity(), identity);
    assert!(empty.selections().is_empty());
    let values = vec![
        selection(
            identity,
            "zeta",
            "/data/zeta",
            VolumeId::new(),
            VolumeAccessMode::ReadOnly,
            VolumeSelectionOrigin::Explicit,
        ),
        selection(
            identity,
            "alpha",
            "/data/alpha",
            VolumeId::new(),
            VolumeAccessMode::ReadWrite,
            VolumeSelectionOrigin::Explicit,
        ),
    ];
    let result = RunVolumeSelections::new(identity, values).unwrap();
    assert_eq!(result.selections()[0].scope().slot().as_str(), "alpha");
    assert_eq!(result.selections()[1].scope().slot().as_str(), "zeta");
    assert_eq!(
        result.selections()[1].scope().access_mode(),
        VolumeAccessMode::ReadOnly
    );
}

#[test]
fn rejects_aliases_duplicates_overlaps_and_foreign_run_before_sorting() {
    let identity = fixture_identity();
    let first = selection(
        identity,
        "data",
        "/data",
        VolumeId::new(),
        VolumeAccessMode::ReadOnly,
        VolumeSelectionOrigin::Explicit,
    );
    assert!(matches!(
        RunVolumeSelections::new(identity, vec![first.clone(), first.clone()]),
        Err(VolumeContractError::DuplicateSlot(_))
    ));
    let alias = selection(
        identity,
        "other",
        "/other",
        first.scope().volume_id(),
        VolumeAccessMode::ReadOnly,
        VolumeSelectionOrigin::Explicit,
    );
    assert!(matches!(
        RunVolumeSelections::new(identity, vec![first.clone(), alias]),
        Err(VolumeContractError::ReusedVolume(_))
    ));
    let child = selection(
        identity,
        "child",
        "/data/child",
        VolumeId::new(),
        VolumeAccessMode::ReadWrite,
        VolumeSelectionOrigin::Explicit,
    );
    assert_eq!(
        RunVolumeSelections::new(identity, vec![first.clone(), child]).unwrap_err(),
        VolumeContractError::OverlappingMountPaths
    );
    assert_eq!(
        RunVolumeSelections::new(fixture_identity(), vec![first]).unwrap_err(),
        VolumeContractError::InvalidRunSelection
    );
}

#[test]
fn exact_scope_and_legacy_declaration_cannot_be_substituted() {
    let identity = fixture_identity();
    let value = selection(
        identity,
        "state",
        LEGACY_STATE_VOLUME_GUEST_PATH,
        VolumeId::new(),
        VolumeAccessMode::ReadWrite,
        VolumeSelectionOrigin::LegacyOrigin,
    );
    assert!(RunVolumeSelections::new(identity, vec![value.clone()]).is_ok());
    let foreign = fixture_identity();
    assert_eq!(
        RunVolumeSelection::new(
            foreign,
            value.scope.clone(),
            value.declaration.clone(),
            value.origin
        )
        .unwrap_err(),
        VolumeContractError::InvalidRunSelection
    );
    let declaration = VolumeSlotDeclaration::new(
        value.declaration.slot().clone(),
        value.declaration.guest_path().clone(),
        VolumeAccessMode::ReadOnly,
        true,
        1,
    )
    .unwrap();
    assert_eq!(
        RunVolumeSelection::new(identity, value.scope.clone(), declaration, value.origin)
            .unwrap_err(),
        VolumeContractError::InvalidRunSelection
    );
    let other = selection(
        identity,
        "extra",
        "/extra",
        VolumeId::new(),
        VolumeAccessMode::ReadWrite,
        VolumeSelectionOrigin::Explicit,
    );
    assert_eq!(
        RunVolumeSelections::new(identity, vec![value, other]).unwrap_err(),
        VolumeContractError::InvalidRunSelection
    );
}

#[test]
fn bounds_and_contract_hash_cannot_be_ignored() {
    let identity = fixture_identity();
    let values = (0..=crate::MAX_VOLUME_SLOTS)
        .map(|index| {
            selection(
                identity,
                &format!("slot{index}"),
                &format!("/data/slot{index}"),
                VolumeId::new(),
                VolumeAccessMode::ReadOnly,
                VolumeSelectionOrigin::Explicit,
            )
        })
        .collect();
    assert!(matches!(
        RunVolumeSelections::new(identity, values),
        Err(VolumeContractError::TooManySlots { .. })
    ));
    let first = selection(
        identity,
        "first",
        "/first",
        VolumeId::new(),
        VolumeAccessMode::ReadOnly,
        VolumeSelectionOrigin::Explicit,
    );
    let mut second = selection(
        identity,
        "second",
        "/second",
        VolumeId::new(),
        VolumeAccessMode::ReadOnly,
        VolumeSelectionOrigin::Explicit,
    );
    second.scope = VolumeMountScope::new(
        identity.instance_id(),
        identity.revision_id(),
        identity.release_agent_id(),
        second.scope.slot().clone(),
        second.scope.volume_id(),
        second.scope.access_mode(),
        AuthorityHash::from_bytes([8; 32]),
    )
    .unwrap();
    assert_eq!(
        RunVolumeSelections::new(identity, vec![first, second]).unwrap_err(),
        VolumeContractError::InvalidRunSelection
    );
    assert!(
        RunVolumeIdentity::new(
            identity.run_id(),
            identity.instance_id(),
            identity.revision_id(),
            identity.release_id(),
            identity.release_agent_id(),
            Uuid::nil()
        )
        .is_err()
    );
}
