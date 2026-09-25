use super::BrokeredRuleCopy;
use super::release_brokered_helpers::{update_contract_diagnostics, validate_brokered_rule_copies};
use super::release_capability_binding::{
    deterministic_requirement_id, release_capability_requirements,
    release_capability_requirements_hash,
};
use super::release_capability_selection::capability_grant_permission;
use agent_config::CapabilitySlotDeclaration;
use authz_domain::Permission;
use capability_domain::{CapabilityOperation, CapabilityResourceKind};
use release_domain::ReleaseAgentId;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

#[test]
fn update_state_contract_diagnostics_are_complete_and_stably_ordered() {
    assert_eq!(
        update_contract_diagnostics(true, false, None),
        vec![
            json!({
                "code": "state_capability_change_unsupported",
                "field": "state_volume.enabled"
            }),
            json!({
                "code": "stateful_update_hook_missing",
                "field": "update_hook"
            }),
        ]
    );
    assert_eq!(
        update_contract_diagnostics(false, true, Some(&json!({"command": "bin/update"}))),
        vec![json!({
            "code": "state_capability_change_unsupported",
            "field": "state_volume.enabled"
        })]
    );
    assert!(
        update_contract_diagnostics(true, true, Some(&json!({"command": "bin/update"}))).is_empty()
    );
}

#[test]
fn broker_rule_copy_validation_requires_exact_one_to_one_coverage() {
    let source = Uuid::new_v4();
    let candidate = Uuid::new_v4();
    let other_source = Uuid::new_v4();
    let sources = BTreeSet::from([source, other_source]);
    let (mapping, diagnostics) = validate_brokered_rule_copies(
        &sources,
        &[BrokeredRuleCopy {
            source_rule_id: source,
            candidate_rule_id: candidate,
        }],
    );
    assert_eq!(mapping.len(), 1);
    assert!(diagnostics.iter().any(|value| {
        value.get("code").and_then(Value::as_str) == Some("brokered_rule_copy_missing")
    }));

    let (_, diagnostics) = validate_brokered_rule_copies(
        &BTreeSet::from([source]),
        &[
            BrokeredRuleCopy {
                source_rule_id: source,
                candidate_rule_id: candidate,
            },
            BrokeredRuleCopy {
                source_rule_id: source,
                candidate_rule_id: Uuid::new_v4(),
            },
        ],
    );
    assert!(diagnostics.iter().any(|value| {
        value.get("code").and_then(Value::as_str) == Some("brokered_rule_copy_duplicate")
    }));

    let (_, diagnostics) = validate_brokered_rule_copies(
        &BTreeSet::from([source]),
        &[BrokeredRuleCopy {
            source_rule_id: Uuid::new_v4(),
            candidate_rule_id: candidate,
        }],
    );
    assert!(diagnostics.iter().any(|value| {
        value.get("code").and_then(Value::as_str) == Some("brokered_rule_copy_source_unavailable")
    }));
}

#[test]
fn release_capability_requirement_identity_and_hash_are_deterministic() {
    let release_agent_id = ReleaseAgentId::new();
    let declarations = vec![CapabilitySlotDeclaration {
        key: String::from("source"),
        purpose: String::from("Read the exact project repository"),
        resource_kind: CapabilityResourceKind::Repository,
        required_operations: vec![CapabilityOperation::GitRead],
        optional_operations: vec![CapabilityOperation::TriggerRun],
        required: true,
        git: None,
    }];
    let first = release_capability_requirements(release_agent_id, &declarations)
        .expect("valid requirement");
    let second = release_capability_requirements(release_agent_id, &declarations)
        .expect("valid requirement");
    assert_eq!(first, second);
    assert_eq!(
        release_capability_requirements_hash(&first),
        release_capability_requirements_hash(&second)
    );
    assert_ne!(
        deterministic_requirement_id(release_agent_id, "source"),
        deterministic_requirement_id(release_agent_id, "output")
    );
}

#[test]
fn capability_grants_require_resource_specific_user_permissions() {
    for (kind, operation, permission) in [
        (
            CapabilityResourceKind::Repository,
            CapabilityOperation::GitRead,
            Permission::CanRead,
        ),
        (
            CapabilityResourceKind::Repository,
            CapabilityOperation::TriggerRun,
            Permission::CanWrite,
        ),
        (
            CapabilityResourceKind::Project,
            CapabilityOperation::Execute,
            Permission::CanWrite,
        ),
        (
            CapabilityResourceKind::Project,
            CapabilityOperation::Recover,
            Permission::CanManage,
        ),
        (
            CapabilityResourceKind::AgentInstance,
            CapabilityOperation::Update,
            Permission::CanUpdate,
        ),
        (
            CapabilityResourceKind::Run,
            CapabilityOperation::Recover,
            Permission::CanRecover,
        ),
        (
            CapabilityResourceKind::StateVolume,
            CapabilityOperation::Attach,
            Permission::CanAttach,
        ),
    ] {
        assert_eq!(
            capability_grant_permission(kind, operation),
            Some(permission)
        );
    }
    assert_eq!(
        capability_grant_permission(
            CapabilityResourceKind::Gateway,
            CapabilityOperation::Execute
        ),
        None
    );
    assert_eq!(
        capability_grant_permission(
            CapabilityResourceKind::Repository,
            CapabilityOperation::Restore
        ),
        None
    );
}
