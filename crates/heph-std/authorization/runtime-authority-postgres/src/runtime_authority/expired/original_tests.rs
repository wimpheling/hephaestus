//! Shape/substitution controls only; no positive persisted original is invented.

use super::*;

fn plan(original: &Original) -> Placement {
    Placement {
        namespace: Uuid::from_u128(8),
        host: "host".into(),
        vm_id: original.id.to_string(),
        consumed_at: Some(OffsetDateTime::UNIX_EPOCH),
        consumption_kind: Some("provision".into()),
        exact: true,
        conflicting: false,
    }
}

#[test]
fn unowned_canonical_unknown_or_conflicting_profiles_cannot_be_legacy() {
    for profile in ["historical_unproven", "owned_canonical", "unknown"] {
        let mut original = controlled_original();
        original.profile = profile.into();
        assert!(!original.strict_legacy());
    }
    let mut original = controlled_original();
    original.legacy_required = false;
    assert!(!original.strict_legacy());
    original = controlled_original();
    original.plan_required = true;
    assert!(!original.strict_legacy());
    original = controlled_original();
    original.namespace = Some(Uuid::from_u128(8).to_string());
    assert!(!original.strict_legacy());
}

#[test]
fn foreign_unconsumed_or_conflicting_placement_denies() {
    let original = controlled_original();
    let mut value = plan(&original);
    let namespace = value.namespace.to_string();
    assert!(!placement_matches(
        &original,
        &value,
        &namespace,
        "other-host"
    ));
    assert!(!placement_matches(
        &original,
        &value,
        &Uuid::from_u128(9).to_string(),
        "host"
    ));
    value.consumed_at = None;
    assert!(!placement_matches(&original, &value, &namespace, "host"));
    value = plan(&original);
    value.consumption_kind = Some("cleanup_only".into());
    assert!(!placement_matches(&original, &value, &namespace, "host"));
    value = plan(&original);
    value.exact = false;
    assert!(!placement_matches(&original, &value, &namespace, "host"));
    value = plan(&original);
    value.conflicting = true;
    assert!(!placement_matches(&original, &value, &namespace, "host"));
}

#[test]
fn changed_original_vm_and_noncanonical_vm_id_deny() {
    let mut original = controlled_original();
    let value = plan(&original);
    original.vm_id = Some("other-original".into());
    assert!(!placement_matches(
        &original,
        &value,
        &value.namespace.to_string(),
        "host"
    ));
    let original = controlled_original();
    let mut value = plan(&original);
    value.vm_id = "another-vm".into();
    assert!(!placement_matches(
        &original,
        &value,
        &value.namespace.to_string(),
        "host"
    ));
}
